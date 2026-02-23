mod handlers;
mod static_files;
mod types;

use anyhow::Result;
use axum::http::{header, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower_http::cors::CorsLayer;

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub ledger_path: Arc<PathBuf>,
    pub encryption_key: Option<Arc<crate::crypto::EncryptionKey>>,
}

pub async fn run(ledger_path: String, port: u16) -> Result<()> {
    if crate::cursor::has_cursor_db() && crate::cursor::is_cache_stale() {
        tokio::spawn(async {
            if let Err(e) = crate::cursor::sync(7).await {
                eprintln!("[vigilo] cursor sync failed: {e}");
            }
        });
    }

    let encryption_key = crate::crypto::load_key();
    let encrypted = encryption_key.is_some();

    let ledger = Arc::new(PathBuf::from(&ledger_path));

    let state = AppState {
        ledger_path: ledger,
        encryption_key: encryption_key.map(Arc::new),
    };

    let (listener, actual_port) = bind_with_fallback(port).await?;

    let cors = CorsLayer::new()
        .allow_origin([
            format!("http://127.0.0.1:{actual_port}").parse().unwrap(),
            format!("http://localhost:{actual_port}").parse().unwrap(),
        ])
        .allow_methods([Method::GET])
        .allow_headers([header::CONTENT_TYPE, header::ACCEPT]);

    let api = Router::new()
        .route("/api/summary", axum::routing::get(handlers::summary))
        .route("/api/sessions", axum::routing::get(handlers::sessions))
        .route("/api/stats", axum::routing::get(handlers::stats))
        .route("/api/events", axum::routing::get(handlers::events))
        .route("/api/errors", axum::routing::get(handlers::errors))
        .route(
            "/api/events/stream",
            axum::routing::get(handlers::event_stream),
        );

    let app = api
        .fallback(static_files::serve)
        .layer(cors)
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(validate_host))
        .with_state(state);

    print_banner(&ledger_path, actual_port, encrypted);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn validate_host(req: Request<axum::body::Body>, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let hostname = host.split(':').next().unwrap_or("");
    if !matches!(hostname, "127.0.0.1" | "localhost" | "[::1]" | "") {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

async fn security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert(
        "Content-Security-Policy",
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'"
            .parse()
            .unwrap(),
    );
    if is_api {
        headers.insert("Cache-Control", "no-store".parse().unwrap());
    }
    response
}

/// Try to bind to the requested port. If it's taken, ask the user whether to
/// pick the next available port.
async fn bind_with_fallback(port: u16) -> Result<(tokio::net::TcpListener, u16)> {
    match tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await {
        Ok(listener) => Ok((listener, port)),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("[vigilo] port {port} is already in use.");
            eprint!("[vigilo] bind to a random available port instead? [Y/n] ");

            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            let answer = answer.trim().to_lowercase();
            if !answer.is_empty() && answer != "y" && answer != "yes" {
                anyhow::bail!("port {port} is already in use — pass --port <N> to choose another");
            }

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let actual_port = listener.local_addr()?.port();
            Ok((listener, actual_port))
        }
        Err(e) => Err(e.into()),
    }
}

fn print_banner(ledger_path: &str, port: u16, encrypted: bool) {
    use crate::models::shorten_home;

    let ledger = Path::new(ledger_path);
    let version = env!("CARGO_PKG_VERSION");
    let url = format!("http://127.0.0.1:{port}");
    let ledger_display = shorten_home(ledger_path);
    let encryption_status = if encrypted {
        "enabled (AES-256-GCM)"
    } else {
        "disabled"
    };

    let ts = handlers::today_summary(ledger);
    let counts = ts.counts;

    // Compute ledger file size
    let ledger_size = std::fs::metadata(ledger_path)
        .map(|m| fmt_bytes(m.len()))
        .unwrap_or_else(|_| "—".to_string());

    // Count active MCP servers
    let mcp_servers = count_mcp_servers();

    let merged = handlers::build_merged_session_list(&ts.sessions);

    // Dim / bold escape codes
    let dim = "\x1b[2m";
    let bold = "\x1b[1m";
    let cyan = "\x1b[36m";
    let yellow = "\x1b[33m";
    let green = "\x1b[32m";
    let red = "\x1b[31m";
    let reset = "\x1b[0m";

    // Header
    let width: usize = 72;
    let quit_hint = format!("{dim}(Ctrl+C to quit){reset}");
    let header = format!("{bold}{cyan}vigilo{reset} {dim}dashboard{reset}");
    // Compute visible length for padding
    let header_visible = "vigilo dashboard".len();
    let quit_visible = "(Ctrl+C to quit)".len();
    let padding = width.saturating_sub(header_visible + quit_visible);
    eprintln!();
    eprintln!("{header}{:>pad$}{quit_hint}", "", pad = padding);
    eprintln!("{dim}{}{reset}", "─".repeat(width));

    // Status rows
    let label_w = 24;
    eprintln!(
        "  {dim}Status{reset}{:>pad$}{bold}{green}online{reset}",
        "",
        pad = label_w - 6
    );
    eprintln!(
        "  {dim}Version{reset}{:>pad$}v{version}",
        "",
        pad = label_w - 7
    );
    eprintln!(
        "  {dim}Dashboard{reset}{:>pad$}{bold}{cyan}{url}{reset}",
        "",
        pad = label_w - 9
    );
    eprintln!(
        "  {dim}Ledger{reset}{:>pad$}{ledger_display}",
        "",
        pad = label_w - 6
    );
    eprintln!(
        "  {dim}Ledger size{reset}{:>pad$}{ledger_size}",
        "",
        pad = label_w - 11
    );
    eprintln!(
        "  {dim}Encryption{reset}{:>pad$}{encryption_status}",
        "",
        pad = label_w - 10
    );
    if mcp_servers > 0 {
        eprintln!(
            "  {dim}MCP servers{reset}{:>pad$}{green}{mcp_servers} active{reset}",
            "",
            pad = label_w - 11
        );
    }
    eprintln!();

    // Today's summary table
    eprintln!("{dim}  Today{reset}");
    eprintln!(
        "{dim}  ├─ Sessions{reset}{:>pad$}{bold}{}{reset}",
        "",
        merged.len(),
        pad = label_w - 12
    );
    eprintln!(
        "{dim}  ├─ Calls{reset}{:>pad$}{bold}{}{reset}",
        "",
        counts.total,
        pad = label_w - 9
    );
    let risk_line = format!(
        "{green}{} read{reset}  {yellow}{} write{reset}  {red}{} exec{reset}",
        counts.reads, counts.writes, counts.execs
    );
    eprintln!(
        "{dim}  ├─ Risk{reset}{:>pad$}{risk_line}",
        "",
        pad = label_w - 8
    );
    if counts.errors > 0 {
        eprintln!(
            "{dim}  ├─ Errors{reset}{:>pad$}{bold}{red}{}{reset}",
            "",
            counts.errors,
            pad = label_w - 10
        );
    }
    if counts.total_cost > 0.0 {
        let cost_str = format_cost(counts.total_cost);
        eprintln!(
            "{dim}  ├─ Cost{reset}{:>pad$}{bold}{yellow}{cost_str}{reset}",
            "",
            pad = label_w - 8
        );
    }
    if counts.total_in > 0 || counts.total_out > 0 {
        let tokens_line = format!(
            "{}in  {}out  {}cache",
            fmt_tk(counts.total_in),
            fmt_tk(counts.total_out),
            fmt_tk(counts.total_cr),
        );
        eprintln!(
            "{dim}  └─ Tokens{reset}{:>pad$}{tokens_line}",
            "",
            pad = label_w - 10
        );
    } else {
        eprintln!(
            "{dim}  └─ Tokens{reset}{:>pad$}{dim}—{reset}",
            "",
            pad = label_w - 10
        );
    }

    eprintln!();
    eprintln!("{dim}{}{reset}", "─".repeat(width));
    eprintln!();
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_cost(usd: f64) -> String {
    if usd < 0.01 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

fn fmt_tk(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M ", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K ", n / 1_000)
    } else if n > 0 {
        format!("{n} ")
    } else {
        String::new()
    }
}

fn count_mcp_servers() -> usize {
    let path = crate::models::mcp_session_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let pid_str = match content.lines().nth(1) {
        Some(s) => s.trim(),
        None => return 0,
    };
    let Ok(pid) = pid_str.parse::<libc::pid_t>() else {
        return 0;
    };
    // Check if the process is alive (signal 0 = existence check)
    if unsafe { libc::kill(pid, 0) } == 0 {
        1
    } else {
        0
    }
}
