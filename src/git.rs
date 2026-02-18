use tokio::process::Command;

async fn git(args: &[&str]) -> Option<String> {
    git_in(args, None).await
}

async fn git_in(args: &[&str], dir: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.output().await.ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

pub async fn root() -> Option<String> {
    git(&["rev-parse", "--show-toplevel"]).await
}

pub async fn name() -> Option<String> {
    name_in(None).await
}

pub async fn name_in(dir: Option<&str>) -> Option<String> {
    let remote = git_in(&["remote", "get-url", "origin"], dir).await?;
    let name = remote
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()?
        .to_string();
    Some(name)
}

pub async fn root_in(dir: &str) -> Option<String> {
    git_in(&["rev-parse", "--show-toplevel"], Some(dir)).await
}

pub async fn branch() -> Option<String> {
    git(&["branch", "--show-current"]).await
}

pub async fn branch_in(dir: &str) -> Option<String> {
    git_in(&["branch", "--show-current"], Some(dir)).await
}

pub async fn commit() -> Option<String> {
    git(&["rev-parse", "--short", "HEAD"]).await
}

pub async fn commit_in(dir: &str) -> Option<String> {
    git_in(&["rev-parse", "--short", "HEAD"], Some(dir)).await
}

pub async fn dirty() -> bool {
    git(&["status", "--porcelain"])
        .await
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

pub async fn dirty_in(dir: &str) -> bool {
    git_in(&["status", "--porcelain"], Some(dir))
        .await
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}
