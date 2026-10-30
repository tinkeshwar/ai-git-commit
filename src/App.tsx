import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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

function App() {
  const [repos, setRepos] = useState<Repo[]>([]);
  const [error, setError] = useState<string | null>(null);
  // path → status, or the error string if `git status` failed
  const [statuses, setStatuses] = useState<Record<string, Status | string>>({});
  const [fetchErrors, setFetchErrors] = useState<Record<string, string>>({});
  const [pendingOnly, setPendingOnly] = useState(false);

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
    setStatuses(Object.fromEntries(entries));
  }

  // Window focus: status only. Refresh: fetch all concurrently, then status.
  useEffect(() => {
    loadStatuses();
    window.addEventListener("focus", loadStatuses);
    return () => window.removeEventListener("focus", loadStatuses);
  }, [repos]);

  async function refresh() {
    const results = await Promise.allSettled(
      repos.map((r) => invoke("fetch", { path: r.path })),
    );
    const errors: Record<string, string> = {};
    results.forEach((res, i) => {
      if (res.status === "rejected") errors[repos[i].path] = String(res.reason);
    });
    setFetchErrors(errors);
    loadStatuses();
  }

  async function addRepo() {
    const path = await open({ directory: true });
    if (path) call("add_repo", { path });
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <button aria-label="Add repo" title="Add repo" onClick={addRepo}>
          +
        </button>
        <button onClick={refresh}>Refresh</button>
        <label>
          <input
            type="checkbox"
            checked={pendingOnly}
            onChange={(e) => setPendingOnly(e.target.checked)}
          />
          Pending only
        </label>
        <ul>
          {repos.map((r) => {
            const s = statuses[r.path];
            const err =
              fetchErrors[r.path] ?? (typeof s === "string" ? s : undefined);
            // hide only repos known to be clean and in sync
            if (pendingOnly && !err && typeof s === "object" && !isPending(s))
              return null;
            return (
              <li key={r.path} title={r.path}>
                <span className="name">{r.name}</span>
                <span className="branch">{r.branch ?? "—"}</span>
                {typeof s === "object" && (
                  <>
                    {s.files.length > 0 && (
                      <span className="badge">{s.files.length}</span>
                    )}
                    {s.ahead > 0 && <span className="badge">↑{s.ahead}</span>}
                    {s.behind > 0 && <span className="badge">↓{s.behind}</span>}
                    {!s.upstream && <span className="badge">no upstream</span>}
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
                <button
                  aria-label={`Remove ${r.name}`}
                  onClick={() => call("remove_repo", { path: r.path })}
                >
                  ×
                </button>
              </li>
            );
          })}
        </ul>
      </aside>
      <main>
        {error && (
          <pre className="error" onClick={() => setError(null)}>
            {error}
          </pre>
        )}
      </main>
    </div>
  );
}

export default App;
