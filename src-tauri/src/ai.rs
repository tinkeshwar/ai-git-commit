use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Serialize)]
pub struct Preset {
    pub id: &'static str,
    /// empty for `custom` (user-entered) and the CLI providers
    pub base_url: &'static str,
    pub needs_key: bool,
    pub model: &'static str,
}

const fn p(id: &'static str, base_url: &'static str, needs_key: bool, model: &'static str) -> Preset {
    Preset { id, base_url, needs_key, model }
}

pub const PRESETS: &[Preset] = &[
    p("openai", "https://api.openai.com/v1", true, "gpt-5-mini"),
    p("anthropic", "https://api.anthropic.com/v1", true, "claude-haiku-5-5"),
    p("gemini", "https://generativelanguage.googleapis.com/v1beta/openai", true, "gemini-2.5-flash"),
    p("groq", "https://api.groq.com/openai/v1", true, "llama-3.3-70b-versatile"),
    p("openrouter", "https://openrouter.ai/api/v1", true, "openai/gpt-5-mini"),
    p("mistral", "https://api.mistral.ai/v1", true, "mistral-small-latest"),
    p("deepseek", "https://api.deepseek.com/v1", true, "deepseek-chat"),
    p("ollama", "http://localhost:11434/v1", false, "llama3.2"),
    p("lmstudio", "http://localhost:1234/v1", false, ""),
    p("custom", "", false, ""),
    p("claude-cli", "", false, ""),
    p("gemini-cli", "", false, ""),
    p("codex-cli", "", false, ""),
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// `ai.json`. Holds keys: never serialize this to the frontend, use `View`.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub provider: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    pub keys: HashMap<String, String>,
}

#[derive(Serialize)]
pub struct View {
    provider: Option<String>,
    model: String,
    base_url: Option<String>,
    cli_path: Option<String>,
    has_key: HashMap<String, bool>,
    presets: &'static [Preset],
}

impl Settings {
    pub fn view(&self) -> View {
        View {
            provider: self.provider.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            cli_path: self.cli_path.clone(),
            has_key: self.keys.keys().map(|k| (k.clone(), true)).collect(),
            presets: PRESETS,
        }
    }

    /// `key: None` keeps the stored key, `Some("")` clears it.
    pub fn set(
        &mut self,
        provider: String,
        model: String,
        base_url: Option<String>,
        cli_path: Option<String>,
        key: Option<String>,
    ) -> Result<(), String> {
        preset(&provider).ok_or(format!("unknown provider {provider}"))?;
        let some = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        match key.map(|k| k.trim().to_string()) {
            Some(k) if k.is_empty() => _ = self.keys.remove(&provider),
            Some(k) => _ = self.keys.insert(provider.clone(), k),
            None => {}
        }
        self.provider = Some(provider);
        self.model = model.trim().to_string();
        self.base_url = some(base_url);
        self.cli_path = some(cli_path);
        Ok(())
    }
}

pub fn load(path: &Path) -> Result<Settings, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("ai.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Written `0600` on unix: the file holds API keys.
pub fn save(path: &Path, s: &Settings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(path).map_err(|e| e.to_string())?;
    // `mode` only applies on create; tighten a pre-existing file before writing the keys
    #[cfg(unix)]
    f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())
}

const FILE_CAP: usize = 4_000;
const TOTAL_CAP: usize = 16_000;
const TRUNCATED: &str = "\n[truncated]\n";

fn is_lockfile(f: &str) -> bool {
    let name = f.rsplit('/').next().unwrap_or(f);
    name.ends_with(".lock") || ["package-lock.json", "pnpm-lock.yaml"].contains(&name)
}

/// First `n` chars of `s`, plus the marker if anything was cut.
fn cap(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}{TRUNCATED}", &s[..i]),
        None => s.to_string(),
    }
}

/// `--stat` in full, then per-file diffs capped per file and in total. Lockfiles and binaries: stat only.
pub fn diff_input(path: &str, files: &[String]) -> Result<String, String> {
    let p = Path::new(path);
    let stat = crate::commit_preview(path.into(), files.to_vec())?;
    let (tracked, untracked) = crate::split_files(p, files)?;
    let head = crate::has_head(p);
    let mut out = format!("{stat}\n");
    let mut left = TOTAL_CAP;
    for (f, new) in tracked.iter().map(|f| (f, false)).chain(untracked.iter().map(|f| (f, true))) {
        if left == 0 {
            break;
        }
        if is_lockfile(f) {
            continue;
        }
        let d = if new {
            crate::run_git_codes(p, &["diff", "--no-index", "--", "/dev/null", f], &[0, 1])?
        } else {
            let one = std::slice::from_ref(f);
            crate::git_files(p, if head { &["diff", "HEAD"] } else { &["diff", "--cached"] }, one)?
        };
        if d.lines().any(|l| l.starts_with("Binary files ") || l == "GIT binary patch") {
            continue;
        }
        let d = cap(&d, FILE_CAP.min(left));
        left = left.saturating_sub(d.chars().count());
        out += &d;
    }
    Ok(out)
}

pub fn build_prompt(stat_and_diff: &str) -> (String, String) {
    let system = "You write git commit messages in the Conventional Commits format.\n\
        First line: `type(scope): summary`, at most 72 characters, imperative mood \
        (type is one of feat, fix, refactor, perf, docs, style, test, build, ci, chore; scope is optional).\n\
        Then, only if useful, a blank line and a short body explaining what and why.\n\
        Reply with the commit message only: plain text, no markdown, no code fences, no quotes.";
    let user = format!("Write a commit message for these changes:\n\n{stat_and_diff}");
    (system.into(), user)
}

/// Model output → `(summary, body)`. Tolerates code fences and wrapping quotes.
pub fn parse_message(text: &str) -> Result<(String, String), String> {
    let mut t = text.trim();
    if let Some(r) = t.strip_prefix("```") {
        // drop the opening fence line (may carry a language tag)
        let r = r.split_once('\n').map_or("", |(_, rest)| rest).trim_end();
        t = r.strip_suffix("```").unwrap_or(r).trim();
    }
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            t = t[1..t.len() - 1].trim();
        }
    }
    let mut lines = t.lines().skip_while(|l| l.trim().is_empty());
    let summary = lines.next().ok_or("the AI returned an empty message")?.trim().to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Ok((summary, body))
}

/// OpenAI-compatible `POST {base_url}/chat/completions` → `choices[0].message.content`.
pub fn chat_http(base_url: &str, key: Option<&str>, model: &str, system: &str, user: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent.post(format!("{}/chat/completions", base_url.trim_end_matches('/')));
    if let Some(k) = key {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "temperature": 0.2,
    });
    let mut res = req.send_json(&body).map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{}: {text}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("bad response: {e}"))?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(String::from)
        .ok_or(format!("no message in response: {text}"))
}

pub const NOT_CONFIGURED: &str = "Configure an AI provider in AI Settings";

/// Prompt goes on stdin, never argv (diffs are large).
fn cli_args(id: &str) -> &'static [&'static str] {
    match id {
        "claude-cli" => &["-p"],
        // gemini: `-p` is "appended to input on stdin", so an empty `-p` means stdin is the whole prompt
        "gemini-cli" => &["-p", ""],
        _ => &["exec", "-"], // codex: `-` reads the prompt from stdin
    }
}

/// `{ "claude-cli": path | null, ... }`. Via a login shell: Finder-launched apps get a minimal PATH.
pub fn detect_ai_clis() -> &'static HashMap<String, Option<String>> {
    static CACHE: OnceLock<HashMap<String, Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let shell = std::env::var("SHELL").unwrap_or("/bin/zsh".into());
        let out = Command::new(shell).args(["-lc", "command -v claude gemini codex"]).output();
        let out = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        // only found names are printed, so map back by basename; skip rc-file noise
        let found: Vec<&str> = out.lines().map(str::trim).filter(|l| l.starts_with('/')).collect();
        ["claude", "gemini", "codex"]
            .iter()
            .map(|n| (format!("{n}-cli"), found.iter().find(|l| l.rsplit('/').next() == Some(n)).map(|l| l.to_string())))
            .collect()
    })
}

const CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `bin args` with `prompt` on stdin; stdout on success, `"<bin> exited <code>: <stderr>"` otherwise.
pub fn chat_cli(bin: &str, args: &[&str], prompt: &str) -> Result<String, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{bin}: {e}"))?;
    // threads so neither a full pipe nor an unread stdin can block the timeout loop
    let mut stdin = child.stdin.take().unwrap();
    let prompt = prompt.to_string();
    std::thread::spawn(move || stdin.write_all(prompt.as_bytes())); // dropped → stdin closed
    let read = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            _ = r.unwrap().read_to_string(&mut s);
            s
        })
    };
    let out = read(child.stdout.take().map(|r| Box::new(r) as _));
    let err = read(child.stderr.take().map(|r| Box::new(r) as _));
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if start.elapsed() > CLI_TIMEOUT {
            _ = child.kill();
            _ = child.wait();
            return Err(format!("{bin} timed out after {}s", CLI_TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if !status.success() {
        let code = status.code().map_or("by signal".into(), |c| c.to_string());
        return Err(format!("{bin} exited {code}: {}", err.trim()));
    }
    Ok(out)
}

/// Settings → configured provider → raw reply text.
pub fn chat(s: &Settings, system: &str, user: &str) -> Result<String, String> {
    let id = s.provider.as_deref().ok_or(NOT_CONFIGURED)?;
    let pr = preset(id).ok_or(NOT_CONFIGURED)?;
    if let Some(name) = id.strip_suffix("-cli") {
        let bin = match s.cli_path.clone() {
            Some(b) => b,
            None => detect_ai_clis().get(id).cloned().flatten().ok_or(format!("{name} CLI not found; set its path in AI Settings"))?,
        };
        return chat_cli(&bin, cli_args(id), &format!("{system}\n\n{user}"));
    }
    let base = if id == "custom" { s.base_url.as_deref().unwrap_or("") } else { pr.base_url };
    let key = s.keys.get(id).map(String::as_str);
    let model = if s.model.is_empty() { pr.model } else { &s.model };
    if base.is_empty() || model.is_empty() || (pr.needs_key && key.is_none()) {
        return Err(NOT_CONFIGURED.into());
    }
    chat_http(base, key, model, system, user)
}

/// Settings → provider call → parsed message.
pub fn generate(s: &Settings, path: &str, files: &[String]) -> Result<(String, String), String> {
    s.provider.as_ref().ok_or(NOT_CONFIGURED)?; // fail before diffing
    let (system, user) = build_prompt(&diff_input(path, files)?);
    parse_message(&chat(s, &system, &user)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    #[test]
    fn settings_round_trip_without_leaking_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.json");
        assert_eq!(load(&path).unwrap(), Settings::default());

        let mut s = Settings::default();
        s.set("openai".into(), "gpt-x".into(), None, None, Some("sk-secret".into())).unwrap();
        s.set("custom".into(), " m ".into(), Some("http://h/v1".into()), None, Some("cust-key".into())).unwrap();
        s.set("custom".into(), "m".into(), Some("http://h/v1".into()), Some("".into()), None).unwrap(); // None keeps
        assert!(s.set("nope".into(), "".into(), None, None, None).is_err());
        save(&path, &s).unwrap();

        let back = load(&path).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.keys["custom"], "cust-key");
        assert_eq!((back.model.as_str(), back.cli_path.as_deref()), ("m", None));

        let view = serde_json::to_string(&back.view()).unwrap();
        assert!(!view.contains("sk-secret") && !view.contains("cust-key"));
        assert!(view.contains(r#""openai":true"#));

        let mut s = back;
        s.set("openai".into(), "".into(), None, None, Some("".into())).unwrap(); // clears
        assert!(!s.keys.contains_key("openai"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            save(&path, &s).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn parse_messages() {
        let ok = |t: &str| parse_message(t).unwrap();
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(ok("feat: add x\n"), s("feat: add x", ""));
        assert_eq!(ok("\n fix(ui): y \n\n why\n more \n"), s("fix(ui): y", "why\n more"));
        assert_eq!(ok("```text\nfeat: z\n\nbody\n```"), s("feat: z", "body"));
        assert_eq!(ok("```\nfeat: z\n```"), s("feat: z", ""));
        assert_eq!(ok("\"chore: q\""), s("chore: q", ""));
        assert_eq!(ok("'chore: q\n\nb'"), s("chore: q", "b"));
        assert!(parse_message("  \n ").is_err());
        assert!(parse_message("```\n```").is_err());
    }

    #[test]
    fn diff_input_caps_and_skips_lockfiles() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let path = p.to_string_lossy().to_string();
        let git = |args: &[&str]| crate::run_git(p, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(p.join("small.txt"), "a\n").unwrap();
        std::fs::write(p.join("Cargo.lock"), "lock-v1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);

        std::fs::write(p.join("small.txt"), "b\n").unwrap();
        std::fs::write(p.join("Cargo.lock"), "lock-v2\n").unwrap();
        std::fs::write(p.join("big.txt"), "x".repeat(50) + &"\nline".repeat(5_000)).unwrap();
        let files: Vec<String> = ["small.txt", "Cargo.lock", "big.txt"].map(String::from).into();
        let out = diff_input(&path, &files).unwrap();

        assert!(out.contains("Cargo.lock") && !out.contains("lock-v2")); // stat only
        assert!(out.contains("big.txt (new)") && out.contains(" small.txt "), "{out}");
        assert!(out.contains("+b") && out.contains("[truncated]"));
        assert!(out.chars().count() < TOTAL_CAP + 1_000);
    }

    /// One-shot HTTP server: returns the raw request it got.
    fn serve_once(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", l.local_addr().unwrap());
        let res = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let (mut req, mut len) = (String::new(), 0);
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                req += &line;
                if line == "\r\n" {
                    break;
                }
            }
            let mut b = vec![0; len];
            r.read_exact(&mut b).unwrap();
            req += &String::from_utf8(b).unwrap();
            s.write_all(res.as_bytes()).unwrap();
            req
        });
        (url, h)
    }

    fn script(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let f = dir.join("fake");
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        f.to_string_lossy().into_owned()
    }

    #[test]
    fn chat_cli_ok_and_error() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(dir.path(), "echo 'feat: x'; echo; cat");
        let out = chat_cli(&ok, &["-p"], "the prompt").unwrap();
        assert_eq!(parse_message(&out).unwrap(), ("feat: x".into(), "the prompt".into()));

        let bad = script(dir.path(), "echo boom >&2; exit 2");
        let e = chat_cli(&bad, &[], "x").unwrap_err();
        assert!(e.contains("exited 2") && e.contains("boom"), "{e}");
    }

    #[test]
    fn chat_http_ok_and_error() {
        let (url, h) = serve_once("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"feat: hi"}}]}"#);
        assert_eq!(chat_http(&url, Some("k1"), "m1", "sys", "usr").unwrap(), "feat: hi");
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /v1/chat/completions"));
        assert!(req.to_ascii_lowercase().contains("authorization: bearer k1"));
        let req: String = req.split_whitespace().collect();
        assert!(req.contains(r#""model":"m1""#) && req.contains(r#""content":"usr""#), "{req}");

        let (url, h) = serve_once("401 Unauthorized", r#"{"error":"bad key"}"#);
        let e = chat_http(&url, None, "m", "s", "u").unwrap_err();
        assert!(e.contains("401") && e.contains("bad key"), "{e}");
        assert!(!h.join().unwrap().to_ascii_lowercase().contains("authorization"));
    }
}
