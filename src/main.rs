mod cli;
mod crypto;
mod cursor_usage;
mod doctor;
mod git;
mod hook;
mod hook_helpers;
mod ledger;
mod models;
mod server;
mod setup;
mod view;

use anyhow::Result;
use cli::{filter_flags, get_flag, parse_date, parse_view_args};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let ledger_path = std::env::var("VIGILO_LEDGER")
        .unwrap_or_else(|_| format!("{}/.vigilo/events.jsonl", models::home()));

    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    if raw_args.iter().any(|a| a == "--no-color") {
        view::fmt::disable_color();
    }

    let args: Vec<String> = raw_args.into_iter().filter(|a| a != "--no-color").collect();

    if args.iter().any(|a| a == "--help" || a == "-h")
        || args.first().map(|s| s.as_str()) == Some("help")
    {
        cli::print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vigilo {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(result) = dispatch_subcommand(&args, &ledger_path).await {
        return result;
    }

    if !args.is_empty() {
        eprintln!("vigilo: unknown command '{}'\n", args[0]);
        eprintln!("Run 'vigilo help' for usage.");
        std::process::exit(1);
    }

    if atty::is(atty::Stream::Stdin) {
        eprintln!("vigilo: running as MCP server, but stdin is a terminal.");
        eprintln!("Did you mean 'vigilo help'?");
        std::process::exit(1);
    }

    let session_id = Uuid::new_v4();
    write_mcp_session_file(&session_id);
    eprintln!("[vigilo] session={session_id}");
    eprintln!("[vigilo] ledger={ledger_path}");

    server::run(ledger_path, session_id).await
}

async fn dispatch_subcommand(args: &[String], ledger_path: &str) -> Option<Result<()>> {
    if matches!(
        args.first().map(|s| s.as_str()),
        Some("view" | "sessions" | "stats" | "errors" | "summary" | "tail" | "diff" | "query")
    ) {
        auto_sync_cursor_cache().await;
    }

    match args.first().map(|s| s.as_str()) {
        Some("view") => Some(view::run(ledger_path, parse_view_args(&args[1..]))),
        Some("generate-key") => Some(generate_key()),
        Some("stats") => Some(dispatch_stats(&args[1..], ledger_path)),
        Some("errors") => Some(dispatch_errors(&args[1..], ledger_path)),
        Some("query") => Some(dispatch_query(&args[1..], ledger_path)),
        Some("diff") => Some(view::diff(ledger_path, &parse_view_args(&args[1..]))),
        Some("cursor-usage") => Some(dispatch_cursor_usage(&args[1..]).await),
        Some("hook") => Some(hook::run(ledger_path).await),
        Some("setup") => Some(setup::run().await),
        Some("watch") => Some(view::watch(ledger_path).await),
        Some("summary") => Some(view::summary(ledger_path)),
        Some("sessions") => Some(view::sessions(ledger_path, parse_view_args(&args[1..]))),
        Some("tail") => Some(dispatch_tail(&args[1..], ledger_path)),
        Some("export") => Some(dispatch_export(&args[1..], ledger_path)),
        Some("prune") => Some(dispatch_prune(&args[1..], ledger_path)),
        Some("doctor") => {
            doctor::run(ledger_path);
            Some(Ok(()))
        }
        _ => None,
    }
}

fn generate_key() -> Result<()> {
    println!("{}", crypto::generate_key_b64());
    Ok(())
}

fn dispatch_stats(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    view::stats_filtered(ledger_path, since.as_deref(), until.as_deref())
}

fn dispatch_errors(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    let expand = args.iter().any(|a| a == "--expand");
    view::errors(ledger_path, since.as_deref(), until.as_deref(), expand)
}

fn dispatch_query(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    let tool = get_flag(args, "--tool");
    let risk = get_flag(args, "--risk");
    let session = get_flag(args, "--session");
    view::query(
        ledger_path,
        since.as_deref(),
        until.as_deref(),
        tool.as_deref(),
        risk.as_deref(),
        session.as_deref(),
    )
}

async fn dispatch_cursor_usage(args: &[String]) -> Result<()> {
    let since = match get_flag(args, "--since-days") {
        Some(s) => s
            .parse()
            .map_err(|_| anyhow::anyhow!("--since-days requires a number, got '{s}'"))?,
        None => 30u32,
    };
    if args.iter().any(|a| a == "--sync") {
        cursor_usage::sync(since).await
    } else {
        cursor_usage::run(since).await
    }
}

fn dispatch_tail(args: &[String], ledger_path: &str) -> Result<()> {
    let n = get_flag(args, "-n")
        .or_else(|| get_flag(args, "--last"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    view::tail(ledger_path, n)
}

fn dispatch_prune(args: &[String], ledger_path: &str) -> Result<()> {
    let days: u32 = get_flag(args, "--older-than")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let removed = ledger::prune(ledger_path, days)?;
    if removed > 0 {
        println!("pruned {removed} rotated ledger file(s) older than {days} days");
    } else {
        println!("no rotated ledger files older than {days} days");
    }
    Ok(())
}

fn dispatch_export(args: &[String], ledger_path: &str) -> Result<()> {
    let format = get_flag(args, "--format").unwrap_or_else(|| "csv".to_string());
    let output = get_flag(args, "--output");
    let filtered: Vec<String> = filter_flags(args, &["--format", "--output"]);
    let view_args = parse_view_args(&filtered);
    view::export(ledger_path, &format, &view_args, output.as_deref())
}

async fn auto_sync_cursor_cache() {
    if !cursor_usage::has_cursor_db() || !cursor_usage::is_cache_stale() {
        return;
    }
    eprintln!("[vigilo] syncing cursor token data...");
    if let Err(e) = cursor_usage::sync(7).await {
        eprintln!("[vigilo] cursor sync failed: {e}");
    }
}

fn write_mcp_session_file(session_id: &Uuid) {
    let content = format!("{}\n{}", session_id, std::process::id());
    let _ = std::fs::write(models::mcp_session_path(), content);
}
