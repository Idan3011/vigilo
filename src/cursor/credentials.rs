use anyhow::{Context, Result};

pub(super) struct Credentials {
    pub user_id: String,
    pub access_token: String,
    pub email: Option<String>,
    pub membership: Option<String>,
}

pub(super) fn read_credentials(db_path: &str) -> Result<Credentials> {
    let conn = super::platform::open_db(db_path)?;

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

pub(super) fn auth_cookie(creds: &Credentials) -> String {
    let raw = format!("{}::{}", creds.user_id, creds.access_token);
    format!("WorkosCursorSessionToken={}", percent_encode(&raw))
}

pub(super) fn percent_encode(s: &str) -> String {
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
