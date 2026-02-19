use tokio::process::Command;

const GIT_TIMEOUT_SECS: u64 = 5;

async fn git(args: &[&str]) -> Option<String> {
    git_in(args, None).await
}

async fn git_in(args: &[&str], dir: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(GIT_TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .ok()?
    .ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    async fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let p = dir.path().to_str().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(p)
            .output()
            .await
            .expect("git init");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(p)
            .output()
            .await
            .expect("git config email");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(p)
            .output()
            .await
            .expect("git config name");
        dir
    }

    async fn make_commit(dir: &std::path::Path) {
        let p = dir.to_str().unwrap();
        fs::write(dir.join("file.txt"), "content").expect("write");
        Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .await
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(p)
            .output()
            .await
            .expect("git commit");
    }

    #[tokio::test]
    async fn root_in_returns_repo_root() {
        let dir = init_repo().await;
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).expect("mkdir");
        let root = root_in(sub.to_str().unwrap()).await;
        assert!(root.is_some());
        assert_eq!(
            root.unwrap(),
            dir.path().canonicalize().unwrap().to_str().unwrap()
        );
    }

    #[tokio::test]
    async fn root_in_returns_none_for_non_repo() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = root_in(dir.path().to_str().unwrap()).await;
        assert!(root.is_none());
    }

    #[tokio::test]
    async fn branch_in_returns_current_branch() {
        let dir = init_repo().await;
        make_commit(dir.path()).await;
        let branch = branch_in(dir.path().to_str().unwrap()).await;
        assert!(branch.is_some());
        let b = branch.unwrap();
        assert!(!b.is_empty());
    }

    #[tokio::test]
    async fn commit_in_returns_short_hash() {
        let dir = init_repo().await;
        make_commit(dir.path()).await;
        let hash = commit_in(dir.path().to_str().unwrap()).await;
        assert!(hash.is_some());
        let h = hash.unwrap();
        assert!(h.len() >= 7 && h.len() <= 12);
    }

    #[tokio::test]
    async fn commit_in_returns_none_without_commits() {
        let dir = init_repo().await;
        let hash = commit_in(dir.path().to_str().unwrap()).await;
        assert!(hash.is_none());
    }

    #[tokio::test]
    async fn dirty_in_false_on_clean_repo() {
        let dir = init_repo().await;
        make_commit(dir.path()).await;
        assert!(!dirty_in(dir.path().to_str().unwrap()).await);
    }

    #[tokio::test]
    async fn dirty_in_true_with_uncommitted_changes() {
        let dir = init_repo().await;
        make_commit(dir.path()).await;
        fs::write(dir.path().join("new.txt"), "dirty").expect("write");
        assert!(dirty_in(dir.path().to_str().unwrap()).await);
    }

    #[tokio::test]
    async fn dirty_in_false_for_non_repo() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(!dirty_in(dir.path().to_str().unwrap()).await);
    }

    #[tokio::test]
    async fn name_in_returns_none_without_remote() {
        let dir = init_repo().await;
        let name = name_in(Some(dir.path().to_str().unwrap())).await;
        assert!(name.is_none());
    }

    #[tokio::test]
    async fn name_in_extracts_from_https_remote() {
        let dir = init_repo().await;
        let p = dir.path().to_str().unwrap();
        Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/user/my-repo.git",
            ])
            .current_dir(p)
            .output()
            .await
            .expect("git remote add");
        let name = name_in(Some(p)).await;
        assert_eq!(name.as_deref(), Some("my-repo"));
    }

    #[tokio::test]
    async fn name_in_extracts_from_ssh_remote() {
        let dir = init_repo().await;
        let p = dir.path().to_str().unwrap();
        Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "git@github.com:user/another-repo.git",
            ])
            .current_dir(p)
            .output()
            .await
            .expect("git remote add");
        let name = name_in(Some(p)).await;
        assert_eq!(name.as_deref(), Some("another-repo"));
    }

    #[tokio::test]
    async fn name_in_handles_trailing_slash() {
        let dir = init_repo().await;
        let p = dir.path().to_str().unwrap();
        Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/user/slash-repo/",
            ])
            .current_dir(p)
            .output()
            .await
            .expect("git remote add");
        let name = name_in(Some(p)).await;
        assert_eq!(name.as_deref(), Some("slash-repo"));
    }
}
