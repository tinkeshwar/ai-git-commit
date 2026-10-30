use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Runs `git <args>` in `repo`. Non-zero exit → `Err(stderr)`.
pub fn run_git(repo: &Path, args: &[&str]) -> Result<String, String> {
    run_git_codes(repo, args, &[0])
}

/// Like `run_git`, but any exit code in `ok` counts as success (`diff --no-index` exits 1 on differences).
pub fn run_git_codes(repo: &Path, args: &[&str], ok: &[i32]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        // no tty in a GUI: fail instead of hanging on a credential prompt
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if out.status.code().is_some_and(|c| ok.contains(&c)) {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileChange>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub status: char,
}

/// Parses `git status --porcelain=v2 --branch`.
pub fn parse_status(out: &str) -> Status {
    let mut s = Status::default();
    for line in out.lines() {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "#" => match rest.split_once(' ') {
                Some(("branch.head", v)) => s.branch = v.into(),
                Some(("branch.upstream", v)) => s.upstream = Some(v.into()),
                Some(("branch.ab", v)) => {
                    let (a, b) = v.split_once(' ').unwrap_or_default();
                    s.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
                    s.behind = b.trim_start_matches('-').parse().unwrap_or(0);
                }
                _ => {}
            },
            // path is the last field: 7 fields precede it for "1", 8 for "2", 9 for "u"
            "1" | "2" | "u" => {
                let n = match kind { "1" => 8, "2" => 9, _ => 10 };
                let fields: Vec<&str> = rest.splitn(n, ' ').collect();
                if let [xy, .., path] = fields.as_slice() {
                    let status = if kind == "u" { 'U' } else { xy.chars().find(|&c| c != '.').unwrap_or('M') };
                    // renames are "new\told"
                    let path = path.split('\t').next().unwrap_or(path).into();
                    s.files.push(FileChange { path, status });
                }
            }
            "?" => s.files.push(FileChange { path: rest.into(), status: '?' }),
            _ => {}
        }
    }
    s
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

    fn files(s: &Status) -> Vec<(&str, char)> {
        s.files.iter().map(|f| (f.path.as_str(), f.status)).collect()
    }

    #[test]
    fn parse_status_fixtures() {
        // clean, tracking, up to date
        let s = parse_status("# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n");
        assert_eq!((s.branch.as_str(), s.upstream.as_deref(), s.ahead, s.behind), ("main", Some("origin/main"), 0, 0));
        assert!(s.files.is_empty());

        // ahead/behind
        let s = parse_status("# branch.head dev\n# branch.upstream origin/dev\n# branch.ab +3 -12\n");
        assert_eq!((s.ahead, s.behind), (3, 12));

        // no upstream
        let s = parse_status("# branch.oid (initial)\n# branch.head main\n");
        assert_eq!((s.upstream, s.ahead, s.behind), (None, 0, 0));

        // dirty: modified (unstaged + staged), added, deleted, rename, untracked, conflict, space in path
        let s = parse_status(concat!(
            "# branch.head main\n",
            "1 .M N... 100644 100644 100644 aaa aaa src/a.rs\n",
            "1 M. N... 100644 100644 100644 aaa bbb my file.txt\n",
            "1 A. N... 000000 100644 100644 000 bbb new.rs\n",
            "1 D. N... 100644 000000 000000 aaa 000 gone.rs\n",
            "2 R. N... 100644 100644 100644 aaa aaa R100 new name.rs\told.rs\n",
            "u UU N... 100644 100644 100644 100644 a b c clash.rs\n",
            "? untracked.txt\n",
        ));
        assert_eq!(
            files(&s),
            [("src/a.rs", 'M'), ("my file.txt", 'M'), ("new.rs", 'A'), ("gone.rs", 'D'), ("new name.rs", 'R'), ("clash.rs", 'U'), ("untracked.txt", '?')]
        );
    }
}
