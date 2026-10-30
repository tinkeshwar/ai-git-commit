use std::path::Path;
use std::process::Command;

/// Runs `git <args>` in `repo`. Non-zero exit → `Err(stderr)`.
pub fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_git_ok_in_repo_err_outside() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run_git(dir.path(), &["rev-parse", "--show-toplevel"]).is_err());
        run_git(dir.path(), &["init"]).unwrap();
        assert!(run_git(dir.path(), &["rev-parse", "--show-toplevel"]).is_ok());
    }
}
