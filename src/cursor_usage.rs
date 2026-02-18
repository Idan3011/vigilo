use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BG_MAGENTA: &str = "\x1b[45m";
const WHITE: &str = "\x1b[97m";

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
             Platform: {:?}\n\
             Searched:\n  {}\n\n\
             Run `vigilo setup` to configure, or set CURSOR_DATA_DIR.",
                detect_platform(),
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
    let dest = "/tmp/vigilo-cursor-state.vscdb";
    std::fs::copy(src, dest).with_context(|| format!("failed to copy {src} → {dest}"))?;
    Ok(dest.to_string())
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
    let access_token =
        query("cursorAuth/accessToken").context("accessToken not found — is Cursor signed in?")?;
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
        .context("statsigBootstrap not found in Cursor DB")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&blob).context("statsigBootstrap is not valid JSON")?;
    parsed["user"]["userID"]
        .as_str()
        .context("userID not found in statsigBootstrap")
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

    eprint!("  {DIM}{} fetching usage data...{RESET}", frames[0]);

    loop {
        let data = fetch_events(client, creds, start_ms, end_ms, page, page_size).await?;
        let events = data["usageEventsDisplay"].as_array();
        match events {
            Some(arr) if !arr.is_empty() => {
                all.extend(arr.iter().cloned());
                let total = data["totalUsageEventsCount"].as_u64().unwrap_or(0);
                let frame = frames[page as usize % frames.len()];
                eprint!(
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
    eprint!(
        "\r  {DIM}✓ fetched {} events{RESET}              \n",
        all.len()
    );
    Ok(all)
}

fn normalize_model(m: &str) -> String {
    match m {
        "default" | "auto" => "Auto".to_string(),
        other => other.to_string(),
    }
}

fn fmt_tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{}K", n / 1_000),
        n => n.to_string(),
    }
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
    println!("  {DIM}billing: {s}{RESET} → {DIM}{e}{RESET}  {DIM}({kind}){RESET}");

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
        println!("  {DIM}plan:{RESET} {BOLD}{used}{RESET}/{limit} requests  {DIM}({remaining} remaining){RESET}  {color}{pct:.0}%{RESET}");
    }

    let od = &summary["individualUsage"]["onDemand"];
    if od["enabled"].as_bool() == Some(true) {
        let used = od["used"].as_u64().unwrap_or(0);
        let limit = od["limit"]
            .as_u64()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        println!("  {DIM}on-demand:{RESET} {BOLD}{used}{RESET} used  {DIM}(limit: {limit}){RESET}");
    }
}

fn print_events(events: &[serde_json::Value], since_days: u32) {
    let mut totals = TokenTotals::default();
    let mut by_model: HashMap<String, TokenTotals> = HashMap::new();

    for ev in events {
        let t = TokenTotals::from_event(ev);
        totals.merge(&t);
        by_model
            .entry(normalize_model(ev["model"].as_str().unwrap_or("unknown")))
            .or_default()
            .merge(&t);
    }

    println!();
    println!("{DIM}── token usage ({since_days}d) ─────────────────────────────{RESET}");
    println!();
    println!("  {BOLD}{}{RESET} requests", totals.count);
    println!("  {CYAN}{}{RESET} input · {CYAN}{}{RESET} output · {DIM}{} cache read · {} cache write{RESET}",
        fmt_tokens(totals.input), fmt_tokens(totals.output),
        fmt_tokens(totals.cache_read), fmt_tokens(totals.cache_write));

    if totals.cost_cents > 0.0 {
        println!(
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
    println!("  {BOLD}by model{RESET}");
    println!("  {DIM}────────{RESET}");
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
        println!(
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
            model: normalize_model(ev["model"].as_str().unwrap_or("unknown")),
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
    println!(
        "  {DIM}synced {} events to {}{RESET}",
        events.len(),
        cache_path()
    );
    Ok(())
}

pub async fn run(since_days: u32) -> Result<()> {
    let db_path = resolve_db_path()?;
    let creds = read_credentials(&db_path)?;

    let badge = format!("{BG_MAGENTA}{BOLD}{WHITE} CURSOR {RESET}");
    let email = creds.email.as_deref().unwrap_or("unknown");
    let membership = creds.membership.as_deref().unwrap_or("unknown");

    println!();
    println!(" {badge}  {BOLD}{email}{RESET}  {DIM}({membership}){RESET}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    eprint!("  {DIM}⠋ connecting to cursor.com...{RESET}");
    match fetch_summary(&client, &creds).await {
        Ok(s) => {
            eprint!("\r                                    \r");
            print_summary(&s);
        }
        Err(e) => {
            eprint!("\r                                    \r");
            eprintln!("  {DIM}usage-summary: {e}{RESET}");
        }
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (since_days as i64 * 86_400_000);
    let events = fetch_all_events(&client, &creds, start_ms, now_ms).await?;

    if events.is_empty() {
        println!("  {DIM}no usage events in the last {since_days} days{RESET}");
    } else {
        print_events(&events, since_days);
        write_cache(&events)?;
    }

    println!();
    Ok(())
}
