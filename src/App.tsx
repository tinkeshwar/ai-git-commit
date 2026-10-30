import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  }, []);

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

  const sel = selected ? statuses[selected] : undefined;
  const selRepo = repos.find((r) => r.path === selected);
  const checked = (selected && picked[selected]) || [];
  const setChecked = (c: string[]) =>
    selected && setPicked({ ...picked, [selected]: c });

  return (
    <div className="app">
      <aside className="sidebar">
        <header className="pane-head">
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
                  className={r.path === selected ? "selected" : ""}
                >
                  <button
                    className="row"
                    title={r.path}
                    onClick={() => {
                      setSelected(r.path);
                      setDiff(null);
                    }}
                  >
                    <span className="repo-name">{r.name}</span>
                    <span className="repo-branch">⎇ {r.branch ?? "—"}</span>
                  </button>
                  <span className="badges">
                    {typeof s === "object" && (
                      <>
                        {s.files.length > 0 && (
                          <span className="badge changes">{s.files.length}</span>
                        )}
                        {s.ahead > 0 && <span className="badge">↑{s.ahead}</span>}
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
                    onClick={() => {
                      if (r.path === selected) setSelected(null);
                      call("remove_repo", { path: r.path });
                    }}
                  >
                    ×
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </aside>

      <section className="files">
        {!selRepo ? (
          <div className="empty">
            <p>Select a repository to see its changes.</p>
          </div>
        ) : typeof sel === "string" ? (
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
                disabled={busy[selected!] || (sel.ahead === 0 && !!sel.upstream)}
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
                {summary.length > 0 && (
                  <span className={summary.length > 72 ? "count over" : "count"}>
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

      <main className="diff-pane">
        {!diff ? (
          <div className="empty">
            <p>Select a file to view its diff.</p>
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
