use anyhow::{Context, Result};

use super::credentials::{auth_cookie, Credentials};
use crate::view::fmt::{ceprint, DIM, RESET};

const SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const EVENTS_URL: &str = "https://cursor.com/api/dashboard/get-filtered-usage-events";

pub(super) async fn fetch_summary(
    client: &reqwest::Client,
    creds: &Credentials,
) -> Result<serde_json::Value> {
    let resp = client
        .get(SUMMARY_URL)
        .header("Cookie", auth_cookie(creds))
        .header("User-Agent", concat!("vigilo/", env!("CARGO_PKG_VERSION")))
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

pub(super) async fn fetch_events(
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
        .header("User-Agent", concat!("vigilo/", env!("CARGO_PKG_VERSION")))
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

pub(super) async fn fetch_all_events(
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
