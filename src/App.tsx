import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./App.css";

type Repo = { path: string; name: string; branch: string | null };
type Status = {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: { path: string; status: string }[];
};

type Preset = {
  id: string;
  base_url: string;
  needs_key: boolean;
  model: string;
};
type AiView = {
  provider: string | null;
  model: string;
  base_url: string | null;
  cli_path: string | null;
  has_key: Record<string, boolean>;
  presets: Preset[];
};
type AiForm = {
  provider: string;
  model: string;
  base_url: string;
  cli_path: string;
  key: string;
  clis: Record<string, string | null>;
  busy?: boolean;
  reply?: string;
  error?: string;
};

const AI_GROUPS: [string, [string, string][]][] = [
  [
    "Cloud",
    [
      ["openai", "OpenAI"],
      ["anthropic", "Anthropic"],
      ["gemini", "Gemini – free tier"],
      ["groq", "Groq – free tier"],
      ["openrouter", "OpenRouter – free models"],
      ["mistral", "Mistral"],
      ["deepseek", "DeepSeek"],
    ],
  ],
  [
    "Local",
    [
      ["ollama", "Ollama"],
      ["lmstudio", "LM Studio"],
    ],
  ],
  ["Custom", [["custom", "Custom (OpenAI-compatible)"]]],
  [
    "CLI",
    [
      ["claude-cli", "Claude Code"],
      ["gemini-cli", "Gemini CLI"],
      ["codex-cli", "Codex"],
    ],
  ],
];

const isCli = (id: string) => id.endsWith("-cli");

// a provider is picked and has a key if it needs one
const aiReady = (v: AiView | null) => {
  const p = v?.presets.find((p) => p.id === v.provider);
  return !!p && (!p.needs_key || !!v!.has_key[p.id]);
};

const isPending = (s: Status) =>
  s.files.length > 0 || s.ahead > 0 || s.behind > 0;

const lineClass = (l: string) =>
  l.startsWith("@@")
    ? "hunk"
    : l[0] === "+"
      ? "add"
      : l[0] === "-"
        ? "del"
        : "";

// "my-git_tool" → "MG", "web" → "WE"
const initials = (name: string) => {
  const words = name.split(/[\s._-]+/).filter(Boolean);
  return (
    words.length > 1 ? words[0][0] + words[1][0] : name.slice(0, 2)
  ).toUpperCase();
};

// stable per-name avatar color
const hue = (name: string) =>
  [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);

function App() {
  const [repos, setRepos] = useState<Repo[]>([]);
  const [error, setError] = useState<string | null>(null);
  // path → status, or the error string if `git status` failed
  const [statuses, setStatuses] = useState<Record<string, Status | string>>({});
  const [fetchErrors, setFetchErrors] = useState<Record<string, string>>({});
  const [pendingOnly, setPendingOnly] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  // repo path → checked file paths (memory only)
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [diff, setDiff] = useState<{ file: string; text: string } | null>(null);
  const [summary, setSummary] = useState("");
  const [body, setBody] = useState("");
  // confirm modal: preview text + what Confirm runs
  const [preview, setPreview] = useState<{
    title: string;
    text: string;
    confirm: () => void;
  } | null>(null);
  // repos with a push/pull in flight
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [refreshing, setRefreshing] = useState(false);
  // sidebar shows full list only while no repo is selected, or when toggled open
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [clone, setClone] = useState<{
    url: string;
    parent: string;
    busy?: boolean;
    error?: string;
  } | null>(null);
  const [ai, setAi] = useState<AiView | null>(null);
  const [aiForm, setAiForm] = useState<AiForm | null>(null);
  // repos with a commit-message generation in flight
  const [generating, setGenerating] = useState<Record<string, boolean>>({});
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const message = body.trim() ? `${summary}\n\n${body}` : summary;

  // Runs a backend call, surfaces git stderr verbatim, then refreshes the list.
  async function call(cmd: string, args?: Record<string, unknown>) {
    try {
      if (cmd !== "list_repos") await invoke(cmd, args);
      setRepos(await invoke<Repo[]>("list_repos"));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    call("list_repos");
    loadAi().catch((e) => setError(String(e)));
  }, []);

  async function loadAi() {
    const v = await invoke<AiView>("get_ai_settings");
    setAi(v);
    return v;
  }

  async function openAiSettings() {
    try {
      const [v, clis] = await Promise.all([
        loadAi(),
        invoke<Record<string, string | null>>("detect_ai_clis"),
      ]);
      const provider = v.provider ?? "openai";
      setAiForm({
        provider,
        model: v.model || presetFor(v, provider)?.model || "",
        base_url: v.base_url ?? "",
        cli_path: v.cli_path ?? "",
        key: "",
        clis,
      });
    } catch (e) {
      setError(String(e));
    }
  }

  const presetFor = (v: AiView | null, id: string) =>
    v?.presets.find((p) => p.id === id);

  async function saveAi(f: AiForm) {
    await invoke("set_ai_settings", {
      provider: f.provider,
      model: f.model,
      baseUrl: f.base_url,
      cliPath: f.cli_path,
      key: f.key || null, // blank keeps the stored key
    });
    await loadAi();
  }

  async function aiAction(f: AiForm, test: boolean) {
    setAiForm({ ...f, busy: true, reply: undefined, error: undefined });
    try {
      await saveAi(f);
      if (!test) return setAiForm(null);
      const reply = await invoke<string>("test_ai");
      setAiForm({
        ...f,
        key: "",
        busy: false,
        reply: reply.trim() || "(empty reply)",
      });
    } catch (e) {
      setAiForm({ ...f, busy: false, error: String(e) });
    }
  }

  async function generateMessage(repo: string) {
    setGenerating((g) => ({ ...g, [repo]: true }));
    try {
      const m = await invoke<{ summary: string; body: string }>(
        "generate_commit_message",
        { path: repo, files: picked[repo] },
      );
      // fields are shared across repos: drop the result if the user moved on
      if (selectedRef.current === repo) {
        setSummary(m.summary);
        setBody(m.body);
      }
    } catch (e) {
      setError(String(e));
    }
    setGenerating((g) => ({ ...g, [repo]: false }));
  }

  async function loadStatuses() {
    const entries = await Promise.all(
      repos.map((r) =>
        invoke<Status>("repo_status", { path: r.path }).then(
          (s) => [r.path, s],
          (e) => [r.path, String(e)],
        ),
      ),
    );
    const next: Record<string, Status | string> = Object.fromEntries(entries);
    setStatuses(next);
    // drop checked files that are no longer changed
    setPicked((prev) =>
      Object.fromEntries(
        Object.entries(prev).map(([repo, files]) => {
          const s = next[repo];
          return [
            repo,
            typeof s === "object"
              ? files.filter((f) => s.files.some((x) => x.path === f))
              : files,
          ];
        }),
      ),
    );
  }

  async function showDiff(repo: string, file: string, untracked: boolean) {
    try {
      const text = await invoke<string>("file_diff", {
        path: repo,
        file,
        untracked,
      });
      setDiff({ file, text });
    } catch (e) {
      setError(String(e));
    }
  }

  async function openPreview(repo: string) {
    try {
      const stat = await invoke<string>("commit_preview", {
        path: repo,
        files: picked[repo],
      });
      setPreview({
        title: "Commit",
        text: `${message}\n\n${stat}`,
        confirm: () => doCommit(repo),
      });
    } catch (e) {
      setError(String(e));
    }
  }

  async function doCommit(repo: string) {
    setPreview(null);
    try {
      await invoke("commit", { path: repo, files: picked[repo], message });
      setSummary("");
      setBody("");
      setPicked({ ...picked, [repo]: [] });
      setDiff(null);
      setError(null);
    } catch (e) {
      setError(String(e)); // keep the message for a retry
    }
    loadStatuses();
  }

  // Network calls: mark the repo busy so its buttons disable meanwhile.
  async function withBusy(repo: string, f: () => Promise<void>) {
    setBusy((b) => ({ ...b, [repo]: true }));
    try {
      await f();
    } catch (e) {
      setError(String(e));
    }
    setBusy((b) => ({ ...b, [repo]: false }));
  }

  // push/pull: preview command → modal → Confirm runs `cmd`
  const openRemote = (repo: string, previewCmd: string, cmd: string) =>
    withBusy(repo, async () => {
      const text = await invoke<string>(previewCmd, { path: repo });
      setPreview({
        title: cmd === "push" ? "Push" : "Pull",
        text: text.trim() || "Nothing to show.",
        confirm: () => doRemote(repo, cmd),
      });
    });

  const doRemote = (repo: string, cmd: string) => {
    setPreview(null);
    return withBusy(repo, async () => {
      await invoke(cmd, { path: repo });
      setError(null);
    }).then(loadStatuses);
  };

  // Window focus: status only. Refresh: fetch all concurrently, then status.
  useEffect(() => {
    loadStatuses();
    window.addEventListener("focus", loadStatuses);
    return () => window.removeEventListener("focus", loadStatuses);
  }, [repos]);

  async function refresh() {
    setRefreshing(true);
    const results = await Promise.allSettled(
      repos.map((r) => invoke("fetch", { path: r.path })),
    );
    const errors: Record<string, string> = {};
    results.forEach((res, i) => {
      if (res.status === "rejected") errors[repos[i].path] = String(res.reason);
    });
    setFetchErrors(errors);
    await loadStatuses();
    setRefreshing(false);
  }

  async function addRepo() {
    const path = await open({ directory: true });
    if (path) call("add_repo", { path });
  }

  function selectRepo(path: string) {
    setSelected(path);
    setDiff(null);
    setSidebarOpen(false);
  }

  function removeRepo(path: string) {
    if (path === selected) {
      setSelected(null);
      setDiff(null);
    }
    call("remove_repo", { path });
  }

  async function pickCloneParent() {
    const parent = await open({ directory: true });
    if (parent && clone) setClone({ ...clone, parent });
  }

  async function doClone() {
    if (!clone) return;
    setClone({ ...clone, busy: true, error: undefined });
    try {
      const path = await invoke<string>("clone_repo", {
        url: clone.url,
        parent: clone.parent,
      });
      setClone(null);
      await call("list_repos");
      selectRepo(path);
    } catch (e) {
      setClone({ ...clone, busy: false, error: String(e) });
    }
  }

  // Native menu items (built in lib.rs) arrive as "menu" events carrying the item id.
  const onMenu = useRef<(id: string) => void>(() => {});
  onMenu.current = (id) => {
    if (id === "add_repo") addRepo();
    else if (id === "clone_repo") setClone({ url: "", parent: "" });
    else if (id === "ai_settings") openAiSettings();
    else if (id === "fetch_all") refresh();
    else if (id === "remove_repo" && selected) removeRepo(selected);
    else if (id === "toggle_sidebar") setSidebarOpen((o) => !o);
  };
  useEffect(() => {
    const un = listen<string>("menu", (e) => onMenu.current(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const sel = selected ? statuses[selected] : undefined;
  const selRepo = repos.find((r) => r.path === selected);
  const checked = (selected && picked[selected]) || [];
  const setChecked = (c: string[]) =>
    selected && setPicked({ ...picked, [selected]: c });

  const collapsed = !!selRepo && !sidebarOpen;

  return (
    <div
      className={`app${selRepo ? " has-repo" : ""}${collapsed ? " collapsed" : ""}`}
    >
      <aside className="sidebar">
        <header className="pane-head">
          {selRepo && (
            <button
              className="icon"
              aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
              title={
                collapsed ? "Expand sidebar (⌘B)" : "Collapse sidebar (⌘B)"
              }
              onClick={() => setSidebarOpen(collapsed)}
            >
              {collapsed ? "☰" : "‹"}
            </button>
          )}
          <h1>Repositories</h1>
          <button
            className="icon"
            aria-label="Fetch all"
            title="Fetch all remotes"
            disabled={refreshing || repos.length === 0}
            onClick={refresh}
          >
            <span className={refreshing ? "spin" : ""}>↻</span>
          </button>
          <button
            className="icon"
            aria-label="Add repo"
            title="Add repository"
            onClick={addRepo}
          >
            +
          </button>
        </header>
        {repos.length > 0 && (
          <label className="toggle">
            <input
              type="checkbox"
              checked={pendingOnly}
              onChange={(e) => setPendingOnly(e.target.checked)}
            />
            Show pending only
          </label>
        )}
        {repos.length === 0 ? (
          <div className="empty">
            <p>No repositories yet.</p>
            <button className="primary" onClick={addRepo}>
              Add repository
            </button>
            <button onClick={() => setClone({ url: "", parent: "" })}>
              Clone repository
            </button>
          </div>
        ) : (
          <ul className="repo-list">
            {repos.map((r) => {
              const s = statuses[r.path];
              const err =
                fetchErrors[r.path] ?? (typeof s === "string" ? s : undefined);
              // hide only repos known to be clean and in sync
              if (pendingOnly && !err && typeof s === "object" && !isPending(s))
                return null;
              return (
                <li
                  key={r.path}
                  className={
                    (r.path === selected ? "selected" : "") +
                    (err
                      ? " has-err"
                      : typeof s === "object" && isPending(s)
                        ? " pending"
                        : "")
                  }
                >
                  <button
                    className="row"
                    title={collapsed ? `${r.name}\n${r.path}` : r.path}
                    onClick={() => selectRepo(r.path)}
                  >
                    <span
                      className="avatar"
                      style={{ "--h": hue(r.name) } as React.CSSProperties}
                    >
                      {initials(r.name)}
                    </span>
                    <span className="repo-text">
                      <span className="repo-name">{r.name}</span>
                      <span className="repo-branch">⎇ {r.branch ?? "—"}</span>
                    </span>
                  </button>
                  <span className="badges">
                    {typeof s === "object" && (
                      <>
                        {s.files.length > 0 && (
                          <span className="badge changes">
                            {s.files.length}
                          </span>
                        )}
                        {s.ahead > 0 && (
                          <span className="badge">↑{s.ahead}</span>
                        )}
                        {s.behind > 0 && (
                          <span className="badge">↓{s.behind}</span>
                        )}
                        {!s.upstream && (
                          <span className="badge muted" title="No upstream">
                            local
                          </span>
                        )}
                      </>
                    )}
                    {err && (
                      <button
                        className="badge err"
                        aria-label={`${r.name} error`}
                        title={err}
                        onClick={() => setError(err)}
                      >
                        !
                      </button>
                    )}
                  </span>
                  <button
                    className="icon remove"
                    aria-label={`Remove ${r.name}`}
                    title="Remove from list"
                    onClick={() => removeRepo(r.path)}
                  >
                    ×
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <footer className="side-foot">
          <button
            className="settings"
            aria-label="Settings"
            title="AI Settings"
            onClick={openAiSettings}
          >
            <span className="gear">⚙</span>
            <span className="label">Settings</span>
          </button>
        </footer>
      </aside>

      {selRepo && (
        <section className="files">
          {typeof sel === "string" ? (
            <div className="empty">
              <p className="err-text">{sel}</p>
            </div>
          ) : !sel ? (
            <div className="empty">
              <span className="spin">↻</span>
            </div>
          ) : (
            <>
              <header className="pane-head repo-head">
                <div className="grow">
                  <h2>{selRepo.name}</h2>
                  <span className="repo-branch">
                    ⎇ {sel.branch}
                    {sel.upstream && ` → ${sel.upstream}`}
                  </span>
                </div>
              </header>
              <div className="sync">
                <button
                  disabled={
                    busy[selected!] || (sel.ahead === 0 && !!sel.upstream)
                  }
                  onClick={() => openRemote(selected!, "push_dry_run", "push")}
                >
                  ↑ Push{sel.ahead > 0 && <b>{sel.ahead}</b>}
                </button>
                <button
                  disabled={busy[selected!] || !sel.upstream}
                  onClick={() => openRemote(selected!, "pull_preview", "pull")}
                >
                  ↓ Pull{sel.behind > 0 && <b>{sel.behind}</b>}
                </button>
                {busy[selected!] && <span className="spin">↻</span>}
              </div>
              {sel.files.length === 0 ? (
                <div className="empty">
                  <p>✓ Working tree clean</p>
                </div>
              ) : (
                <>
                  <label className="toggle all">
                    <input
                      type="checkbox"
                      checked={checked.length === sel.files.length}
                      ref={(el) => {
                        if (el)
                          el.indeterminate =
                            checked.length > 0 &&
                            checked.length < sel.files.length;
                      }}
                      onChange={(e) =>
                        setChecked(
                          e.target.checked ? sel.files.map((f) => f.path) : [],
                        )
                      }
                    />
                    {sel.files.length} changed file
                    {sel.files.length === 1 ? "" : "s"}
                  </label>
                  <ul className="file-list">
                    {sel.files.map((f) => {
                      const slash = f.path.lastIndexOf("/");
                      return (
                        <li
                          key={f.path}
                          className={diff?.file === f.path ? "selected" : ""}
                          title={f.path}
                          onClick={() =>
                            showDiff(selected!, f.path, f.status === "?")
                          }
                        >
                          <input
                            type="checkbox"
                            aria-label={`Select ${f.path}`}
                            checked={checked.includes(f.path)}
                            onClick={(e) => e.stopPropagation()}
                            onChange={(e) =>
                              setChecked(
                                e.target.checked
                                  ? [...checked, f.path]
                                  : checked.filter((c) => c !== f.path),
                              )
                            }
                          />
                          <span className="file-name">
                            {f.path.slice(slash + 1)}
                            {slash > 0 && (
                              <span className="dir">
                                {f.path.slice(0, slash)}
                              </span>
                            )}
                          </span>
                          <span className="status" data-s={f.status}>
                            {f.status === "?" ? "U" : f.status}
                          </span>
                        </li>
                      );
                    })}
                  </ul>
                </>
              )}
              <form
                className="commit"
                onSubmit={(e) => {
                  e.preventDefault();
                  openPreview(selected!);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey))
                    e.currentTarget.requestSubmit();
                }}
              >
                <div className="summary">
                  <input
                    placeholder="Summary (required)"
                    aria-label="Commit summary"
                    value={summary}
                    onChange={(e) => setSummary(e.target.value)}
                  />
                  <button
                    type="button"
                    className="gen"
                    aria-label="Generate commit message"
                    title={
                      aiReady(ai)
                        ? "Generate commit message"
                        : "Set up AI Settings first"
                    }
                    disabled={
                      !aiReady(ai) || !checked.length || generating[selected!]
                    }
                    onClick={() => generateMessage(selected!)}
                  >
                    <span className={generating[selected!] ? "twinkle" : ""}>
                      ✨
                    </span>
                    AI
                  </button>
                  {summary.length > 0 && (
                    <span
                      className={summary.length > 72 ? "count over" : "count"}
                    >
                      {72 - summary.length}
                    </span>
                  )}
                </div>
                <textarea
                  placeholder="Description"
                  aria-label="Commit description"
                  value={body}
                  onChange={(e) => setBody(e.target.value)}
                />
                <button
                  className="primary"
                  title="⌘↵"
                  disabled={!summary.trim() || !checked.length}
                >
                  Commit {checked.length} file{checked.length === 1 ? "" : "s"}
                </button>
              </form>
            </>
          )}
        </section>
      )}

      <main className="diff-pane">
        {!diff ? (
          <div className="empty">
            {selRepo ? (
              <p>Select a file to view its diff.</p>
            ) : repos.length > 0 ? (
              <p>Select a repository to see its changes.</p>
            ) : (
              <>
                <h2>Welcome</h2>
                <p>Add a local repository or clone one to get started.</p>
              </>
            )}
          </div>
        ) : (
          <>
            <header className="pane-head diff-head">
              <span className="mono">{diff.file}</span>
              <button
                className="icon"
                aria-label="Close diff"
                onClick={() => setDiff(null)}
              >
                ×
              </button>
            </header>
            {/^Binary files .* differ$/m.test(diff.text) ? (
              <div className="empty">
                <p>Binary file — no text diff.</p>
              </div>
            ) : (
              <pre className="diff">
                {diff.text.split("\n").map((l, i) => (
                  <div key={i} className={lineClass(l)}>
                    {l || " "}
                  </div>
                ))}
              </pre>
            )}
          </>
        )}
      </main>

      {error && (
        <div className="toast" role="alert">
          <pre>{error}</pre>
          <button
            className="icon"
            aria-label="Dismiss error"
            onClick={() => setError(null)}
          >
            ×
          </button>
        </div>
      )}

      {clone && (
        <dialog
          ref={(d) => {
            if (d && !d.open) d.showModal();
          }}
          onClose={() => setClone(null)}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              doClone();
            }}
          >
            <h3>Clone repository</h3>
            <div className="fields">
              <label>
                Repository URL
                <input
                  autoFocus
                  placeholder="https://github.com/org/repo.git"
                  value={clone.url}
                  disabled={clone.busy}
                  onChange={(e) => setClone({ ...clone, url: e.target.value })}
                />
              </label>
              <label>
                Location
                <span className="pick">
                  <input
                    readOnly
                    placeholder="Choose a parent folder…"
                    value={clone.parent}
                    onClick={pickCloneParent}
                  />
                  <button
                    type="button"
                    disabled={clone.busy}
                    onClick={pickCloneParent}
                  >
                    Choose…
                  </button>
                </span>
              </label>
              {clone.error && <pre className="err-text">{clone.error}</pre>}
            </div>
            <footer>
              <button type="button" onClick={() => setClone(null)}>
                Cancel
              </button>
              <button
                className="primary"
                disabled={clone.busy || !clone.url.trim() || !clone.parent}
              >
                {clone.busy && <span className="spin">↻</span>}
                {clone.busy ? "Cloning…" : "Clone"}
              </button>
            </footer>
          </form>
        </dialog>
      )}

      {aiForm &&
        (() => {
          const f = aiForm;
          const p = presetFor(ai, f.provider);
          const set = (patch: Partial<AiForm>) =>
            setAiForm({ ...f, ...patch, reply: undefined, error: undefined });
          return (
            <dialog
              ref={(d) => {
                if (d && !d.open) d.showModal();
              }}
              onClose={() => setAiForm(null)}
            >
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  aiAction(f, false);
                }}
              >
                <h3>AI Settings</h3>
                <div className="fields">
                  <label>
                    Provider
                    <select
                      value={f.provider}
                      disabled={f.busy}
                      onChange={(e) =>
                        set({
                          provider: e.target.value,
                          model: presetFor(ai, e.target.value)?.model ?? "",
                          key: "",
                        })
                      }
                    >
                      {AI_GROUPS.map(([group, items]) => (
                        <optgroup key={group} label={group}>
                          {items.map(([id, label]) => {
                            const missing = isCli(id) && !f.clis[id];
                            return (
                              <option key={id} value={id} disabled={missing}>
                                {label}
                                {missing ? " (not found)" : ""}
                              </option>
                            );
                          })}
                        </optgroup>
                      ))}
                    </select>
                  </label>
                  {!isCli(f.provider) && (
                    <label>
                      Model
                      <input
                        value={f.model}
                        disabled={f.busy}
                        onChange={(e) => set({ model: e.target.value })}
                      />
                    </label>
                  )}
                  {p?.needs_key && (
                    <label>
                      API key
                      <input
                        type="password"
                        placeholder={ai?.has_key[f.provider] ? "saved" : ""}
                        value={f.key}
                        disabled={f.busy}
                        onChange={(e) => set({ key: e.target.value })}
                      />
                    </label>
                  )}
                  {f.provider === "custom" && (
                    <label>
                      Base URL
                      <input
                        placeholder="https://host/v1"
                        value={f.base_url}
                        disabled={f.busy}
                        onChange={(e) => set({ base_url: e.target.value })}
                      />
                    </label>
                  )}
                  {isCli(f.provider) && (
                    <label>
                      CLI path
                      <input
                        placeholder={f.clis[f.provider] ?? "auto-detect"}
                        value={f.cli_path}
                        disabled={f.busy}
                        onChange={(e) => set({ cli_path: e.target.value })}
                      />
                    </label>
                  )}
                  {f.reply && <pre className="ok">{f.reply}</pre>}
                  {f.error && <pre className="err-text">{f.error}</pre>}
                </div>
                <footer>
                  <button type="button" onClick={() => setAiForm(null)}>
                    Cancel
                  </button>
                  <button
                    type="button"
                    disabled={f.busy}
                    onClick={() => aiAction(f, true)}
                  >
                    {f.busy && <span className="spin">↻</span>}
                    Test
                  </button>
                  <button className="primary" disabled={f.busy}>
                    Save
                  </button>
                </footer>
              </form>
            </dialog>
          );
        })()}

      {preview && (
        <dialog
          ref={(d) => {
            if (d && !d.open) d.showModal();
          }}
          onClose={() => setPreview(null)}
        >
          <h3>{preview.title}</h3>
          <pre>{preview.text}</pre>
          <footer>
            <button onClick={() => setPreview(null)}>Cancel</button>
            <button className="primary" autoFocus onClick={preview.confirm}>
              {preview.title}
            </button>
          </footer>
        </dialog>
      )}
    </div>
  );
}

export default App;
