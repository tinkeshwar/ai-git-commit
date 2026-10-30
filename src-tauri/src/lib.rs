mod git;

use git::run_git;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_repos, add_repo, remove_repo])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
