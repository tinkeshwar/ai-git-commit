mod git;

use git::{parse_status, run_git, run_git_codes, Status};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
struct Repo {
    path: String,
    name: String,
    branch: Option<String>,
}

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("repos.json"))
}

fn load(app: &AppHandle) -> Result<Vec<String>, String> {
    match std::fs::read_to_string(store_path(app)?) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("repos.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

fn save(app: &AppHandle, repos: &[String]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(repos).map_err(|e| e.to_string())?;
    std::fs::write(store_path(app)?, json).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_repos(app: AppHandle) -> Result<Vec<Repo>, String> {
    Ok(load(&app)?
        .into_iter()
        .map(|path| {
            let p = Path::new(&path);
            Repo {
                name: p.file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned()),
                // None for missing folders or repos with no commits yet
                branch: run_git(p, &["rev-parse", "--abbrev-ref", "HEAD"])
                    .ok()
                    .map(|b| b.trim().to_string()),
                path,
            }
        })
        .collect())
}

#[tauri::command]
fn add_repo(app: AppHandle, path: String) -> Result<(), String> {
    let root = run_git(Path::new(&path), &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_string();
    let mut repos = load(&app)?;
    if repos.contains(&root) {
        return Err(format!("{root} is already added"));
    }
    repos.push(root);
    save(&app, &repos)
}

#[tauri::command]
fn remove_repo(app: AppHandle, path: String) -> Result<(), String> {
    let mut repos = load(&app)?;
    repos.retain(|r| r != &path);
    save(&app, &repos)
}

#[tauri::command(async)]
fn repo_status(path: String) -> Result<Status, String> {
    let p = Path::new(&path);
    let mut s = parse_status(&run_git(p, &["status", "--porcelain=v2", "--branch"])?);
    if s.upstream.is_none() {
        // no upstream: every local commit counts as unpushed (fails → 0 before the first commit)
        s.ahead = run_git(p, &["rev-list", "--count", "HEAD"])
            .ok()
            .and_then(|c| c.trim().parse().ok())
            .unwrap_or(0);
    }
    Ok(s)
}

#[tauri::command(async)]
fn fetch(path: String) -> Result<(), String> {
    run_git(Path::new(&path), &["fetch", "--prune"]).map(|_| ())
}

#[tauri::command(async)]
fn file_diff(path: String, file: String, untracked: bool) -> Result<String, String> {
    let p = Path::new(&path);
    if untracked {
        return run_git_codes(p, &["diff", "--no-index", "--", "/dev/null", &file], &[0, 1]);
    }
    if run_git(p, &["rev-parse", "--verify", "HEAD"]).is_err() {
        // no commits yet: everything tracked is staged-new
        return run_git(p, &["diff", "--cached", "--", &file]);
    }
    run_git(p, &["diff", "HEAD", "--", &file])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_repos, add_repo, remove_repo, repo_status, fetch, file_diff])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_diff_cases() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let path = p.to_string_lossy().to_string();
        let git = |args: &[&str]| run_git(p, args).unwrap();
        let diff = |f: &str, u| file_diff(path.clone(), f.into(), u).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        assert!(diff("a.txt", false).contains("+one")); // no commits yet
        std::fs::write(p.join("b.txt"), "bee\n").unwrap();
        std::fs::write(p.join("c.txt"), "sea\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);

        std::fs::write(p.join("a.txt"), "two\n").unwrap();
        std::fs::remove_file(p.join("b.txt")).unwrap();
        git(&["mv", "c.txt", "d.txt"]);
        std::fs::write(p.join("new.txt"), "fresh\n").unwrap();
        assert!(diff("a.txt", false).contains("-one\n+two"));
        assert!(diff("b.txt", false).contains("-bee"));
        assert!(diff("d.txt", false).contains("+sea"));
        assert!(diff("new.txt", true).contains("+fresh"));
    }
}
