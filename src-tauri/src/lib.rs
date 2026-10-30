mod ai;
mod git;

use git::{parse_status, run_git, run_git_codes, Status};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};

#[derive(Serialize)]
struct Repo {
    path: String,
    name: String,
    branch: Option<String>,
}

fn store_path(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

fn load(app: &AppHandle) -> Result<Vec<String>, String> {
    match std::fs::read_to_string(store_path(app, "repos.json")?) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("repos.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

fn save(app: &AppHandle, repos: &[String]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(repos).map_err(|e| e.to_string())?;
    std::fs::write(store_path(app, "repos.json")?, json).map_err(|e| e.to_string())
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

/// `https://host/org/name.git/` → `name`
fn clone_dir_name(url: &str) -> Option<&str> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then_some(name)
}

/// Clones `url` into `parent/<name>`, adds it to the list, returns the new path.
#[tauri::command(async)]
fn clone_repo(app: AppHandle, url: String, parent: String) -> Result<String, String> {
    let name = clone_dir_name(url.trim()).ok_or("can't derive a folder name from that URL")?;
    let dest = Path::new(&parent).join(name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    run_git(Path::new(&parent), &["clone", "--", url.trim(), name])?;
    let dest = dest.to_string_lossy().into_owned();
    add_repo(app, dest.clone())?;
    Ok(dest)
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

// `--literal-pathspecs`: file names are paths, never globs
fn git_files(p: &Path, args: &[&str], files: &[String]) -> Result<String, String> {
    let mut a = vec!["--literal-pathspecs"];
    a.extend(args);
    a.push("--");
    a.extend(files.iter().map(String::as_str));
    run_git(p, &a)
}

fn has_head(p: &Path) -> bool {
    run_git(p, &["rev-parse", "--verify", "HEAD"]).is_ok()
}

/// `files` → `(tracked, untracked)`
fn split_files(p: &Path, files: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let untracked = git_files(p, &["ls-files", "--others", "--exclude-standard"], files)?;
    let untracked: Vec<String> = untracked.lines().map(String::from).collect();
    let tracked = files.iter().filter(|f| !untracked.contains(f)).cloned().collect();
    Ok((tracked, untracked))
}

#[tauri::command(async)]
fn commit_preview(path: String, files: Vec<String>) -> Result<String, String> {
    let p = Path::new(&path);
    let (tracked, untracked) = split_files(p, &files)?;
    // an empty pathspec would diff everything
    let mut out = if tracked.is_empty() {
        String::new()
    } else if has_head(p) {
        git_files(p, &["diff", "HEAD", "--stat"], &tracked)?
    } else {
        git_files(p, &["diff", "--cached", "--stat"], &tracked)?
    };
    for f in untracked {
        out += &format!(" {f} (new)\n");
    }
    Ok(out)
}

#[tauri::command(async)]
fn commit(path: String, files: Vec<String>, message: String) -> Result<(), String> {
    let p = Path::new(&path);
    git_files(p, &["add", "-A"], &files)?;
    // pathspec commit: only these files, even if others are staged
    git_files(p, &["commit", "-m", &message], &files).map(|_| ())
}

// no upstream → `-u origin <branch>` so the first push sets it
fn git_push(p: &Path, dry: bool) -> Result<String, String> {
    let branch = run_git(p, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let mut a = vec!["push", "--porcelain"];
    if dry {
        a.push("--dry-run");
    }
    if run_git(p, &["rev-parse", "--abbrev-ref", "@{u}"]).is_err() {
        a.extend(["-u", "origin", &branch]);
    }
    run_git(p, &a).map_err(|e| {
        if e.contains("failed to push some refs") { format!("{e}\n\nRejected: pull first.") } else { e }
    })
}

#[tauri::command(async)]
fn push_dry_run(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    let refs = git_push(p, true)?;
    let log = match run_git(p, &["log", "--oneline", "@{u}..HEAD"]) {
        Ok(l) => l,
        Err(_) => format!("will set upstream\n{}", run_git(p, &["log", "--oneline", "-50", "HEAD"])?),
    };
    Ok(format!("{refs}\n{log}"))
}

#[tauri::command(async)]
fn push(path: String) -> Result<(), String> {
    git_push(Path::new(&path), false).map(|_| ())
}

#[tauri::command(async)]
fn pull_preview(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    run_git(p, &["fetch"])?;
    let log = run_git(p, &["log", "--oneline", "HEAD..@{u}"])?;
    let stat = run_git(p, &["diff", "--stat", "HEAD...@{u}"])?;
    let diverged = run_git(p, &["rev-list", "--count", "@{u}..HEAD"])?.trim() != "0";
    let warn = if diverged { "Diverged: ff-only pull will be refused.\n\n" } else { "" };
    Ok(format!("{warn}{log}\n{stat}"))
}

#[tauri::command(async)]
fn pull(path: String) -> Result<(), String> {
    run_git(Path::new(&path), &["pull", "--ff-only"]).map(|_| ()).map_err(|e| {
        if e.contains("fast-forward") { format!("{e}\n\nff-only refused: branch has diverged, resolve in terminal.") } else { e }
    })
}

#[tauri::command]
fn get_ai_settings(app: AppHandle) -> Result<ai::View, String> {
    Ok(ai::load(&store_path(&app, "ai.json")?)?.view())
}

#[tauri::command]
fn set_ai_settings(
    app: AppHandle,
    provider: String,
    model: String,
    base_url: Option<String>,
    cli_path: Option<String>,
    key: Option<String>,
) -> Result<(), String> {
    let path = store_path(&app, "ai.json")?;
    let mut s = ai::load(&path)?;
    s.set(provider, model, base_url, cli_path, key)?;
    ai::save(&path, &s)
}

#[tauri::command(async)]
fn detect_ai_clis() -> std::collections::HashMap<String, Option<String>> {
    ai::detect_ai_clis().clone()
}

#[tauri::command(async)]
fn test_ai(app: AppHandle) -> Result<String, String> {
    ai::chat(&ai::load(&store_path(&app, "ai.json")?)?, "You are a connectivity check.", "Reply with: ok")
}

#[derive(Serialize)]
struct CommitMessage {
    summary: String,
    body: String,
}

#[tauri::command(async)]
fn generate_commit_message(app: AppHandle, path: String, files: Vec<String>) -> Result<CommitMessage, String> {
    let s = ai::load(&store_path(&app, "ai.json")?)?;
    let (summary, body) = ai::generate(&s, &path, &files)?;
    Ok(CommitMessage { summary, body })
}

/// Platform default menu (keeps Edit → copy/paste) with our items prepended to File / View.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    let item = |id: &str, text: &str, key: Option<&str>| MenuItem::with_id(app, id, text, true, key);
    let sub = |name: &str| -> tauri::Result<Option<Submenu<Wry>>> {
        Ok(menu.items()?.into_iter().find_map(|i| {
            i.as_submenu().filter(|s| s.text().is_ok_and(|t| t == name)).cloned()
        }))
    };
    let file = match sub("File")? {
        Some(f) => f,
        None => {
            let f = Submenu::new(app, "File", true)?;
            menu.prepend(&f)?;
            f
        }
    };
    file.prepend_items(&[
        &item("add_repo", "Add Repository…", Some("CmdOrCtrl+O"))?,
        &item("clone_repo", "Clone Repository…", Some("CmdOrCtrl+Shift+O"))?,
        &item("ai_settings", "AI Settings…", None)?,
        &PredefinedMenuItem::separator(app)?,
        &item("fetch_all", "Fetch All", Some("CmdOrCtrl+Shift+F"))?,
        &item("remove_repo", "Remove Repository from List", None)?,
        &PredefinedMenuItem::separator(app)?,
    ])?;
    let toggle = item("toggle_sidebar", "Toggle Sidebar", Some("CmdOrCtrl+B"))?;
    match sub("View")? {
        Some(v) => v.prepend(&toggle)?,
        None => file.insert(&toggle, 5)?,
    }
    Ok(menu)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Finder-launched apps get PATH=/usr/bin:/bin:..., missing Homebrew git, git-lfs, `gh` credential helpers.
/// Adopt the login shell's PATH; keep the inherited one if the shell fails.
#[cfg(target_os = "macos")]
fn import_login_path() {
    let shell = std::env::var("SHELL").unwrap_or("/bin/zsh".into());
    let Ok(out) = std::process::Command::new(shell).args(["-lc", "printf %s \"$PATH\""]).output() else { return };
    let path = String::from_utf8_lossy(&out.stdout);
    // rc files may print noise first; PATH is the last line
    if let Some(path) = path.lines().last().filter(|p| p.contains('/')) {
        // SAFETY: called first thing in `run`, before any other thread exists
        unsafe { std::env::set_var("PATH", path) };
    }
}

pub fn run() {
    #[cfg(target_os = "macos")]
    import_login_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.set_menu(build_menu(app.handle())?)?;
            // the frontend switches on the item id; predefined items have random ids and are ignored
            app.on_menu_event(|app, e| {
                let _ = app.emit("menu", e.id().0.as_str());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_repos, add_repo, clone_repo, remove_repo, repo_status, fetch, file_diff, commit_preview, commit, push_dry_run, push, pull_preview, pull, get_ai_settings, set_ai_settings, generate_commit_message, detect_ai_clis, test_ai])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_dir_names() {
        assert_eq!(clone_dir_name("https://github.com/a/b.git"), Some("b"));
        assert_eq!(clone_dir_name("https://github.com/a/b/"), Some("b"));
        assert_eq!(clone_dir_name("git@github.com:a/b.git"), Some("b"));
        assert_eq!(clone_dir_name("git@host:b.git"), Some("b"));
        assert_eq!(clone_dir_name(".git"), None);
    }

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

    #[test]
    fn commit_only_picked_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let path = p.to_string_lossy().to_string();
        let git = |args: &[&str]| run_git(p, args).unwrap();
        let v = |fs: &[&str]| fs.iter().map(|f| f.to_string()).collect::<Vec<_>>();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(p.join("first[1].txt"), "x\n").unwrap();
        commit(path.clone(), v(&["first[1].txt"]), "init".into()).unwrap(); // first commit, glob chars
        for f in ["a", "b", "c"] {
            std::fs::write(p.join(f), "1\n").unwrap();
        }
        git(&["add", "a", "b", "c"]);
        git(&["commit", "-qm", "abc"]);

        std::fs::write(p.join("a"), "2\n").unwrap();
        std::fs::write(p.join("b"), "2\n").unwrap();
        git(&["add", "b"]); // pre-staged outside the tool
        std::fs::remove_file(p.join("c")).unwrap();
        std::fs::write(p.join("new"), "n\n").unwrap();

        let preview = commit_preview(path.clone(), v(&["a", "c", "new"])).unwrap();
        assert!(preview.contains("a ") && preview.contains("c ") && preview.contains("new (new)"));
        commit(path.clone(), v(&["a", "c", "new"]), "sum\n\nbody".into()).unwrap();

        assert_eq!(git(&["log", "-1", "--format=%B"]).trim(), "sum\n\nbody");
        assert_eq!(git(&["show", "--name-only", "--format="]).trim(), "a\nc\nnew");
        assert_eq!(git(&["status", "--porcelain"]), "M  b\n");
    }

    #[test]
    fn push_with_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (remote, a, b) = (dir.path().join("r.git"), dir.path().join("a"), dir.path().join("b"));
        run_git(dir.path(), &["init", "-q", "--bare", "r.git"]).unwrap();
        for d in [&a, &b] {
            run_git(dir.path(), &["clone", "-q", remote.to_str().unwrap(), d.to_str().unwrap()]).unwrap();
            run_git(d, &["config", "user.email", "t@t"]).unwrap();
            run_git(d, &["config", "user.name", "t"]).unwrap();
        }
        let git = |d: &Path, args: &[&str]| run_git(d, args).unwrap();
        let s = |d: &Path| d.to_string_lossy().to_string();

        // no upstream yet: preview flags it, push sets it
        git(&a, &["commit", "-q", "--allow-empty", "-m", "c1"]);
        let pv = push_dry_run(s(&a)).unwrap();
        assert!(pv.contains("will set upstream") && pv.contains("c1"));
        push(s(&a)).unwrap();
        assert!(git(&a, &["rev-parse", "--abbrev-ref", "@{u}"]).contains("origin/"));

        // upstream set: preview lists exactly the unpushed commits
        git(&a, &["commit", "-q", "--allow-empty", "-m", "two"]);
        git(&a, &["commit", "-q", "--allow-empty", "-m", "three"]);
        let pv = push_dry_run(s(&a)).unwrap();
        assert!(pv.contains("two") && pv.contains("three") && !pv.contains("c1"));
        push(s(&a)).unwrap();
        assert_eq!(repo_status(s(&a)).unwrap().ahead, 0);

        // b is behind and diverged: rejected with a hint
        git(&b, &["pull", "-q"]);
        git(&a, &["commit", "-q", "--allow-empty", "-m", "four"]);
        push(s(&a)).unwrap();
        git(&b, &["commit", "-q", "--allow-empty", "-m", "five"]);
        assert!(push(s(&b)).unwrap_err().contains("pull first"));
    }

    #[test]
    fn pull_with_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (remote, a, b) = (dir.path().join("r.git"), dir.path().join("a"), dir.path().join("b"));
        run_git(dir.path(), &["init", "-q", "--bare", "r.git"]).unwrap();
        for d in [&a, &b] {
            run_git(dir.path(), &["clone", "-q", remote.to_str().unwrap(), d.to_str().unwrap()]).unwrap();
            run_git(d, &["config", "user.email", "t@t"]).unwrap();
            run_git(d, &["config", "user.name", "t"]).unwrap();
        }
        let git = |d: &Path, args: &[&str]| run_git(d, args).unwrap();
        let s = |d: &Path| d.to_string_lossy().to_string();

        git(&a, &["commit", "-q", "--allow-empty", "-m", "c1"]);
        push(s(&a)).unwrap();
        git(&b, &["pull", "-q"]);

        // b behind by two: preview lists exactly those, pull fast-forwards
        std::fs::write(a.join("f.txt"), "x\n").unwrap();
        git(&a, &["add", "f.txt"]);
        git(&a, &["commit", "-q", "-m", "two"]);
        git(&a, &["commit", "-q", "--allow-empty", "-m", "three"]);
        push(s(&a)).unwrap();
        let pv = pull_preview(s(&b)).unwrap();
        assert!(pv.contains("two") && pv.contains("three") && pv.contains("f.txt") && !pv.contains("c1"));
        assert!(!pv.contains("Diverged"));
        pull(s(&b)).unwrap();
        assert_eq!(repo_status(s(&b)).unwrap().behind, 0);

        // diverged: warned, refused, no merge commit
        git(&a, &["commit", "-q", "--allow-empty", "-m", "four"]);
        push(s(&a)).unwrap();
        git(&b, &["commit", "-q", "--allow-empty", "-m", "five"]);
        assert!(pull_preview(s(&b)).unwrap().contains("Diverged"));
        assert!(pull(s(&b)).unwrap_err().contains("ff-only refused"));
        assert_eq!(git(&b, &["log", "-1", "--format=%s"]).trim(), "five");
    }
}
