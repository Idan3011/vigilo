use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use crate::view::fmt::{
    ceprint, ceprintln, cprintln, fmt_tokens, normalize_model, BG_MAGENTA, BOLD, CYAN, DIM, GREEN,
    RED, RESET, WHITE, YELLOW,
};

#[derive(Debug, PartialEq)]
enum Platform {
    Linux,
    MacOs,
    Windows,
    Wsl,
}

fn detect_platform() -> Platform {
    if cfg!(target_os = "windows") {
        return Platform::Windows;
    }
    if cfg!(target_os = "macos") {
        return Platform::MacOs;
    }
    if is_wsl() {
        return Platform::Wsl;
    }
    Platform::Linux
}

fn platform_name() -> &'static str {
    match detect_platform() {
        Platform::Linux => "Linux",
        Platform::MacOs => "macOS",
        Platform::Windows => "Windows",
        Platform::Wsl => "WSL",
    }
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

const DB_SUFFIX: &str = "User/globalStorage/state.vscdb";

pub fn resolve_db_path() -> Result<String> {
    if let Ok(dir) = std::env::var("CURSOR_DATA_DIR") {
        let path = format!("{dir}/{DB_SUFFIX}");
        return require_exists(&path, "CURSOR_DATA_DIR points to a missing DB");
    }

    if let Some(path) = read_config_key("CURSOR_DB") {
        if Path::new(&path).exists() {
            return Ok(path);
        }
    }

    discover_db()
}

pub fn discover_db() -> Result<String> {
    let candidates = candidate_paths();

    candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cursor database not found.\n\
             Platform: {}\n\
             Searched:\n  {}\n\n\
             Run `vigilo setup` to configure, or set CURSOR_DATA_DIR.",
                platform_name(),
                candidates.join("\n  ")
            )
        })
}

fn candidate_paths() -> Vec<String> {
    let home = home_dir();
    match detect_platform() {
        Platform::Wsl => wsl_candidates(&home),
        Platform::MacOs => vec![format!(
            "{home}/Library/Application Support/Cursor/{DB_SUFFIX}"
        )],
        Platform::Windows => windows_candidates(),
        Platform::Linux => vec![format!("{home}/.config/Cursor/{DB_SUFFIX}")],
    }
}

fn windows_candidates() -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        paths.push(format!("{appdata}/Cursor/{DB_SUFFIX}"));
    }
    paths
}

fn wsl_candidates(home: &str) -> Vec<String> {
    let mount = wsl_mount_root();
    let mut paths = Vec::new();

    if let Some(user) = wsl_windows_username() {
        paths.push(format!(
            "{mount}/Users/{user}/AppData/Roaming/Cursor/{DB_SUFFIX}"
        ));
    }

    let users_dir = format!("{mount}/Users");
    if let Ok(entries) = std::fs::read_dir(&users_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_system_user(&name) {
                continue;
            }
            let candidate = format!(
                "{}/AppData/Roaming/Cursor/{DB_SUFFIX}",
                entry.path().display()
            );
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    paths.push(format!("{home}/.config/Cursor/{DB_SUFFIX}"));

    paths
}

fn is_system_user(name: &str) -> bool {
    matches!(name, "Default" | "Public" | "Default User" | "All Users")
}

fn wsl_mount_root() -> String {
    if let Some(path) = run_command("wslpath", &["-u", "C:\\"]) {
        return path.trim_end_matches('/').to_string();
    }

    if let Ok(conf) = std::fs::read_to_string("/etc/wsl.conf") {
        for line in conf.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("root") {
                if let Some(val) = trimmed
                    .split_once('=')
                    .map(|(_, v)| v.trim().trim_matches('/'))
                {
                    if !val.is_empty() {
                        return format!("/{val}/c");
                    }
                }
            }
        }
    }

    "/mnt/c".to_string()
}

fn wsl_windows_username() -> Option<String> {
    run_command("wslvar", &["USERNAME"]).or_else(|| {
        run_command("cmd.exe", &["/c", "echo", "%USERNAME%"]).filter(|s| !s.contains('%'))
    })
}

fn run_command(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn require_exists(path: &str, hint: &str) -> Result<String> {
    if Path::new(path).exists() {
        Ok(path.to_string())
    } else {
        Err(anyhow::anyhow!("{hint}: {path}"))
    }
}

fn open_db(path: &str) -> Result<rusqlite::Connection> {
    let effective = if needs_local_copy(path) {
        copy_to_local(path)?
    } else {
        path.to_string()
    };

    rusqlite::Connection::open_with_flags(&effective, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open Cursor DB at {path}"))
}

fn needs_local_copy(path: &str) -> bool {
    path.starts_with("/mnt/")
}

fn copy_to_local(src: &str) -> Result<String> {
    let dest = format!("{}/.vigilo/cursor-state.vscdb", home_dir());
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, &dest).with_context(|| format!("failed to copy {src} → {dest}"))?;
    Ok(dest)
}

struct Credentials {
    user_id: String,
    access_token: String,
    email: Option<String>,
    membership: Option<String>,
}

fn read_credentials(db_path: &str) -> Result<Credentials> {
    let conn = open_db(db_path)?;

    let query = |key: &str| -> Option<String> {
        conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .ok()
    };

    let user_id = extract_user_id(&query)?;
    let access_token = query("cursorAuth/accessToken")
        .context("Could not read auth token — is Cursor signed in?")?;
    let email = query("cursorAuth/cachedEmail");
    let membership = query("cursorAuth/stripeMembershipType");

    Ok(Credentials {
        user_id,
        access_token,
        email,
        membership,
    })
}

fn extract_user_id(query: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let blob = query("workbench.experiments.statsigBootstrap")
        .context("Could not find user ID in Cursor database")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&blob).context("Could not parse user data from Cursor database")?;
    parsed["user"]["userID"]
        .as_str()
        .context("User ID missing — your Cursor installation may be unsupported")
        .map(|s| s.to_string())
}

const SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const EVENTS_URL: &str = "https://cursor.com/api/dashboard/get-filtered-usage-events";

fn auth_cookie(creds: &Credentials) -> String {
    let raw = format!("{}::{}", creds.user_id, creds.access_token);
    format!("WorkosCursorSessionToken={}", percent_encode(&raw))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn fetch_summary(client: &reqwest::Client, creds: &Credentials) -> Result<serde_json::Value> {
    let resp = client
        .get(SUMMARY_URL)
        .header("Cookie", auth_cookie(creds))
        .header("User-Agent", "vigilo/0.1")
        .send()
        .await
        .context("failed to reach cursor.com/api/usage-summary")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("usage-summary returned {status}: {body}"));
    }
    resp.json().await.context("invalid JSON from usage-summary")
}

async fn fetch_events(
    client: &reqwest::Client,
    creds: &Credentials,
    start_ms: i64,
    end_ms: i64,
    page: u32,
    page_size: u32,
) -> Result<serde_json::Value> {
    let body = serde_json::json!({
        "teamId": 0,
        "startDate": start_ms.to_string(),
        "endDate": end_ms.to_string(),
        "page": page,
        "pageSize": page_size,
    });

    let resp = client
        .post(EVENTS_URL)
        .header("Cookie", auth_cookie(creds))
        .header("User-Agent", "Mozilla/5.0 (compatible; vigilo/0.1)")
        .header("Origin", "https://cursor.com")
        .header("Referer", "https://cursor.com/settings")
        .json(&body)
        .send()
        .await
        .context("failed to reach cursor.com/api/dashboard/get-filtered-usage-events")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "filtered-usage-events returned {status}: {body}"
        ));
    }
    resp.json()
        .await
        .context("invalid JSON from filtered-usage-events")
}

async fn fetch_all_events(
    client: &reqwest::Client,
    creds: &Credentials,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<serde_json::Value>> {
    let mut all = Vec::new();
    let mut page = 1u32;
    let page_size = 100u32;
    let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    ceprint!("  {DIM}{} fetching usage data...{RESET}", frames[0]);

    loop {
        let data = fetch_events(client, creds, start_ms, end_ms, page, page_size).await?;
        let events = data["usageEventsDisplay"].as_array();
        match events {
            Some(arr) if !arr.is_empty() => {
                all.extend(arr.iter().cloned());
                let total = data["totalUsageEventsCount"].as_u64().unwrap_or(0);
                let frame = frames[page as usize % frames.len()];
                ceprint!(
                    "\r  {DIM}{frame} fetching usage data... {}/{total}{RESET}  ",
                    all.len()
                );
                if all.len() as u64 >= total {
                    break;
                }
                page += 1;
            }
            _ => break,
        }
    }
    ceprint!(
        "\r  {DIM}✓ fetched {} events{RESET}              \n",
        all.len()
    );
    Ok(all)
}

fn fmt_cost_cents(cents: f64) -> String {
    let usd = cents / 100.0;
    match usd {
        usd if usd < 0.01 => format!("${usd:.4}"),
        usd if usd < 1.0 => format!("${usd:.3}"),
        usd => format!("${usd:.2}"),
    }
}

fn print_summary(summary: &serde_json::Value) {
    let start = summary["billingCycleStart"].as_str().unwrap_or("?");
    let end = summary["billingCycleEnd"].as_str().unwrap_or("?");
    let kind = summary["limitType"].as_str().unwrap_or("unknown");

    let s = start.get(5..10).unwrap_or(start);
    let e = end.get(5..10).unwrap_or(end);
    cprintln!("  {DIM}billing: {s}{RESET} → {DIM}{e}{RESET}  {DIM}({kind}){RESET}");

    let plan = &summary["individualUsage"]["plan"];
    if plan.is_object() {
        let used = plan["used"].as_u64().unwrap_or(0);
        let limit = plan["limit"].as_u64().unwrap_or(0);
        let remaining = plan["remaining"].as_u64().unwrap_or(0);
        let pct = plan["totalPercentUsed"].as_f64().unwrap_or(0.0);
        let color = if pct > 80.0 {
            RED
        } else if pct > 50.0 {
            YELLOW
        } else {
            GREEN
        };
        cprintln!("  {DIM}plan:{RESET} {BOLD}{used}{RESET}/{limit} requests  {DIM}({remaining} remaining){RESET}  {color}{pct:.0}%{RESET}");
    }

    let od = &summary["individualUsage"]["onDemand"];
    if od["enabled"].as_bool() == Some(true) {
        let used = od["used"].as_u64().unwrap_or(0);
        let limit = od["limit"]
            .as_u64()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        cprintln!(
            "  {DIM}on-demand:{RESET} {BOLD}{used}{RESET} used  {DIM}(limit: {limit}){RESET}"
        );
    }
}

fn print_events(events: &[serde_json::Value], since_days: u32) {
    let mut totals = TokenTotals::default();
    let mut by_model: HashMap<String, TokenTotals> = HashMap::new();

    for ev in events {
        let t = TokenTotals::from_event(ev);
        totals.merge(&t);
        by_model
            .entry(normalize_model(ev["model"].as_str().unwrap_or("unknown")).to_string())
            .or_default()
            .merge(&t);
    }

    println!();
    cprintln!("{DIM}── token usage ({since_days}d) ─────────────────────────────{RESET}");
    println!();
    cprintln!("  {BOLD}{}{RESET} requests", totals.count);
    cprintln!("  {CYAN}{}{RESET} input · {CYAN}{}{RESET} output · {DIM}{} cache read · {} cache write{RESET}",
        fmt_tokens(totals.input), fmt_tokens(totals.output),
        fmt_tokens(totals.cache_read), fmt_tokens(totals.cache_write));

    if totals.cost_cents > 0.0 {
        cprintln!(
            "  {YELLOW}{}{RESET} total cost",
            fmt_cost_cents(totals.cost_cents)
        );
    }

    print_model_breakdown(by_model);
}

fn print_model_breakdown(by_model: HashMap<String, TokenTotals>) {
    let mut models: Vec<(String, TokenTotals)> = by_model.into_iter().collect();
    models.sort_by(|a, b| b.1.count.cmp(&a.1.count));

    println!();
    cprintln!("  {BOLD}by model{RESET}");
    cprintln!("  {DIM}────────{RESET}");
    for (model, t) in &models {
        let cost = if t.cost_cents > 0.0 {
            format!(" · {}", fmt_cost_cents(t.cost_cents))
        } else {
            String::new()
        };
        let cache = if t.cache_read > 0 {
            format!(" · cache:{}", fmt_tokens(t.cache_read))
        } else {
            String::new()
        };
        cprintln!(
            "  {BOLD}{:>4}×{RESET} {model}  {DIM}{} in · {} out{cache}{cost}{RESET}",
            t.count,
            fmt_tokens(t.input),
            fmt_tokens(t.output)
        );
    }
}

#[derive(Default)]
struct TokenTotals {
    count: usize,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost_cents: f64,
}

impl TokenTotals {
    fn from_event(ev: &serde_json::Value) -> Self {
        let tok = &ev["tokenUsage"];
        Self {
            count: 1,
            input: tok["inputTokens"].as_u64().unwrap_or(0),
            output: tok["outputTokens"].as_u64().unwrap_or(0),
            cache_read: tok["cacheReadTokens"].as_u64().unwrap_or(0),
            cache_write: tok["cacheWriteTokens"].as_u64().unwrap_or(0),
            cost_cents: tok["totalCents"].as_f64().unwrap_or(0.0),
        }
    }

    fn merge(&mut self, other: &Self) {
        self.count += other.count;
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost_cents += other.cost_cents;
    }
}

fn read_config_key(key: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let config = std::fs::read_to_string(format!("{home}/.vigilo/config")).ok()?;
    config
        .lines()
        .find(|line| {
            let k = line.split('=').next().unwrap_or("").trim();
            k == key
        })
        .and_then(|line| line.split_once('='))
        .map(|(_, val)| val.trim().to_string())
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".into())
}

const CACHE_FILE: &str = ".vigilo/cursor-tokens.jsonl";

fn cache_path() -> String {
    format!("{}/{CACHE_FILE}", home_dir())
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
    fn from_api(ev: &serde_json::Value) -> Option<Self> {
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

fn write_cache(events: &[serde_json::Value]) -> Result<()> {
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

pub struct CachedSessionTokens {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub request_count: usize,
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
            age > 3600
        }
        Err(_) => true,
    }
}

pub fn has_cursor_db() -> bool {
    resolve_db_path().is_ok()
}

pub async fn sync(since_days: u32) -> Result<()> {
    let db_path = resolve_db_path()?;
    let creds = read_credentials(&db_path)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (since_days as i64 * 86_400_000);
    let events = fetch_all_events(&client, &creds, start_ms, now_ms).await?;

    write_cache(&events)?;
    cprintln!(
        "  {DIM}synced {} events to {}{RESET}",
        events.len(),
        cache_path()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_system_user_filters_known() {
        assert!(is_system_user("Default"));
        assert!(is_system_user("Public"));
        assert!(is_system_user("Default User"));
        assert!(is_system_user("All Users"));
    }

    #[test]
    fn is_system_user_allows_normal() {
        assert!(!is_system_user("john"));
        assert!(!is_system_user("admin"));
    }

    #[test]
    fn needs_local_copy_mnt_paths() {
        assert!(needs_local_copy("/mnt/c/Users/foo/state.vscdb"));
        assert!(!needs_local_copy("/home/user/.config/Cursor/state.vscdb"));
    }

    #[test]
    fn percent_encode_leaves_unreserved() {
        assert_eq!(
            percent_encode("hello-world_v1.0~test"),
            "hello-world_v1.0~test"
        );
    }

    #[test]
    fn percent_encode_encodes_special_chars() {
        assert_eq!(percent_encode("a::b"), "a%3A%3Ab");
        assert_eq!(percent_encode("foo bar"), "foo%20bar");
    }

    #[test]
    fn auth_cookie_format() {
        let creds = Credentials {
            user_id: "user123".to_string(),
            access_token: "tok456".to_string(),
            email: None,
            membership: None,
        };
        let cookie = auth_cookie(&creds);
        assert!(cookie.starts_with("WorkosCursorSessionToken="));
        assert!(cookie.contains("user123"));
        assert!(cookie.contains("tok456"));
    }

    #[test]
    fn auth_cookie_encodes_colons() {
        let creds = Credentials {
            user_id: "u".to_string(),
            access_token: "t".to_string(),
            email: None,
            membership: None,
        };
        let cookie = auth_cookie(&creds);
        assert!(cookie.contains("%3A%3A"));
    }

    #[test]
    fn fmt_cost_cents_small() {
        assert_eq!(fmt_cost_cents(0.5), "$0.0050");
    }

    #[test]
    fn fmt_cost_cents_medium() {
        assert_eq!(fmt_cost_cents(50.0), "$0.500");
    }

    #[test]
    fn fmt_cost_cents_large() {
        assert_eq!(fmt_cost_cents(1234.0), "$12.34");
    }

    #[test]
    fn token_totals_from_event() {
        let ev = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "tokenUsage": {
                "inputTokens": 1000,
                "outputTokens": 500,
                "cacheReadTokens": 200,
                "cacheWriteTokens": 50,
                "totalCents": 3.5
            }
        });
        let t = TokenTotals::from_event(&ev);
        assert_eq!(t.count, 1);
        assert_eq!(t.input, 1000);
        assert_eq!(t.output, 500);
        assert_eq!(t.cache_read, 200);
        assert_eq!(t.cache_write, 50);
        assert!((t.cost_cents - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn token_totals_from_event_missing_fields() {
        let ev = serde_json::json!({ "model": "unknown" });
        let t = TokenTotals::from_event(&ev);
        assert_eq!(t.input, 0);
        assert_eq!(t.output, 0);
    }

    #[test]
    fn token_totals_merge() {
        let mut a = TokenTotals {
            count: 2,
            input: 100,
            output: 50,
            cache_read: 10,
            cache_write: 5,
            cost_cents: 1.0,
        };
        let b = TokenTotals {
            count: 1,
            input: 200,
            output: 100,
            cache_read: 20,
            cache_write: 10,
            cost_cents: 2.0,
        };
        a.merge(&b);
        assert_eq!(a.count, 3);
        assert_eq!(a.input, 300);
        assert_eq!(a.output, 150);
        assert_eq!(a.cache_read, 30);
        assert_eq!(a.cache_write, 15);
        assert!((a.cost_cents - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cached_token_event_from_api() {
        let ev = serde_json::json!({
            "timestamp": "1708300000000",
            "model": "claude-sonnet-4-20250514",
            "tokenUsage": {
                "inputTokens": 800,
                "outputTokens": 400,
                "cacheReadTokens": 100,
                "cacheWriteTokens": 25,
                "totalCents": 2.0
            }
        });
        let cached = CachedTokenEvent::from_api(&ev).unwrap();
        assert_eq!(cached.timestamp_ms, 1708300000000);
        assert_eq!(cached.input_tokens, 800);
        assert_eq!(cached.output_tokens, 400);
        assert_eq!(cached.cache_read_tokens, 100);
        assert_eq!(cached.cache_write_tokens, 25);
    }

    #[test]
    fn cached_token_event_from_api_missing_timestamp() {
        let ev = serde_json::json!({ "model": "test" });
        assert!(CachedTokenEvent::from_api(&ev).is_none());
    }

    #[test]
    fn cached_token_event_from_api_invalid_timestamp() {
        let ev = serde_json::json!({ "timestamp": "not-a-number" });
        assert!(CachedTokenEvent::from_api(&ev).is_none());
    }

    #[test]
    fn aggregate_cached_tokens_empty() {
        assert!(aggregate_cached_tokens(&[]).is_none());
    }

    #[test]
    fn aggregate_cached_tokens_sums_correctly() {
        let events = vec![
            CachedTokenEvent {
                timestamp_ms: 1000,
                model: "sonnet".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
                cache_write_tokens: 5,
                cost_cents: 1.0,
            },
            CachedTokenEvent {
                timestamp_ms: 2000,
                model: "sonnet".to_string(),
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 20,
                cache_write_tokens: 10,
                cost_cents: 2.0,
            },
            CachedTokenEvent {
                timestamp_ms: 3000,
                model: "opus".to_string(),
                input_tokens: 50,
                output_tokens: 25,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_cents: 3.0,
            },
        ];
        let agg = aggregate_cached_tokens(&events).unwrap();
        assert_eq!(agg.input_tokens, 350);
        assert_eq!(agg.output_tokens, 175);
        assert_eq!(agg.cache_read_tokens, 30);
        assert_eq!(agg.request_count, 3);
        assert!((agg.cost_usd - 0.06).abs() < 0.001);
        assert_eq!(agg.model, "sonnet");
    }

    #[test]
    fn cached_token_event_round_trips_through_json() {
        let event = CachedTokenEvent {
            timestamp_ms: 1708300000000,
            model: "claude-sonnet-4-20250514".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 50,
            cost_cents: 3.5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: CachedTokenEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timestamp_ms, event.timestamp_ms);
        assert_eq!(parsed.model, event.model);
        assert_eq!(parsed.input_tokens, event.input_tokens);
        assert_eq!(parsed.output_tokens, event.output_tokens);
    }
}

pub async fn run(since_days: u32) -> Result<()> {
    let db_path = resolve_db_path()?;
    let creds = read_credentials(&db_path)?;

    let badge = format!("{BG_MAGENTA}{BOLD}{WHITE} CURSOR {RESET}");
    let email = creds.email.as_deref().unwrap_or("unknown");
    let membership = creds.membership.as_deref().unwrap_or("unknown");

    println!();
    cprintln!(" {badge}  {BOLD}{email}{RESET}  {DIM}({membership}){RESET}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    ceprint!("  {DIM}⠋ connecting to cursor.com...{RESET}");
    match fetch_summary(&client, &creds).await {
        Ok(s) => {
            eprint!("\r                                    \r");
            print_summary(&s);
        }
        Err(e) => {
            eprint!("\r                                    \r");
            ceprintln!("  {DIM}usage-summary: {e}{RESET}");
        }
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (since_days as i64 * 86_400_000);
    let events = fetch_all_events(&client, &creds, start_ms, now_ms).await?;

    if events.is_empty() {
        cprintln!("  {DIM}no usage events in the last {since_days} days{RESET}");
    } else {
        print_events(&events, since_days);
        write_cache(&events)?;
    }

    println!();
    Ok(())
}
