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
    text: string;
    confirm: () => void;
  } | null>(null);
  // repos with a push/pull in flight
  const [busy, setBusy] = useState<Record<string, boolean>>({});
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
      setPreview({ text, confirm: () => doRemote(repo, cmd) });
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
              <li
                key={r.path}
                title={r.path}
                className={r.path === selected ? "selected" : ""}
              >
                <span
                  className="name"
                  onClick={() => {
                    setSelected(r.path);
                    setDiff(null);
                  }}
                >
                  {r.name}
                </span>
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
      {selected && typeof statuses[selected] === "object" && (
        <section className="files">
          {(() => {
            const files = (statuses[selected] as Status).files;
            const checked = picked[selected] ?? [];
            const setChecked = (c: string[]) =>
              setPicked({ ...picked, [selected]: c });
            return (
              <>
                {(() => {
                  const s = statuses[selected] as Status;
                  return (
                    <>
                      <button
                        disabled={
                          busy[selected] || (s.ahead === 0 && !!s.upstream)
                        }
                        onClick={() =>
                          openRemote(selected, "push_dry_run", "push")
                        }
                      >
                        Push ↑{s.ahead}
                      </button>
                      <button
                        disabled={busy[selected] || !s.upstream}
                        onClick={() =>
                          openRemote(selected, "pull_preview", "pull")
                        }
                      >
                        Pull ↓{s.behind}
                      </button>
                      {busy[selected] && <span>Working…</span>}
                    </>
                  );
                })()}
                <label>
                  <input
                    type="checkbox"
                    checked={
                      files.length > 0 && checked.length === files.length
                    }
                    onChange={(e) =>
                      setChecked(
                        e.target.checked ? files.map((f) => f.path) : [],
                      )
                    }
                  />
                  {files.length} changed
                </label>
                <ul>
                  {files.map((f) => (
                    <li
                      key={f.path}
                      className={diff?.file === f.path ? "selected" : ""}
                    >
                      <input
                        type="checkbox"
                        aria-label={`Select ${f.path}`}
                        checked={checked.includes(f.path)}
                        onChange={(e) =>
                          setChecked(
                            e.target.checked
                              ? [...checked, f.path]
                              : checked.filter((c) => c !== f.path),
                          )
                        }
                      />
                      <span className="status">{f.status}</span>
                      <span
                        className="name"
                        title={f.path}
                        onClick={() =>
                          showDiff(selected, f.path, f.status === "?")
                        }
                      >
                        {f.path}
                      </span>
                    </li>
                  ))}
                </ul>
              </>
            );
          })()}
        </section>
      )}
      <main>
        {error && (
          <pre className="error" onClick={() => setError(null)}>
            {error}
          </pre>
        )}
        {diff &&
          (/^Binary files .* differ$/m.test(diff.text) ? (
            <p>Binary file</p>
          ) : (
            <pre className="diff">
              {diff.text.split("\n").map((l, i) => (
                <div key={i} className={lineClass(l)}>
                  {l || " "}
                </div>
              ))}
            </pre>
          ))}
        {selected && typeof statuses[selected] === "object" && (
          <form
            className="commit"
            onSubmit={(e) => {
              e.preventDefault();
              openPreview(selected);
            }}
          >
            <input
              placeholder="Summary"
              aria-label="Commit summary"
              value={summary}
              onChange={(e) => setSummary(e.target.value)}
            />
            <textarea
              placeholder="Description"
              aria-label="Commit description"
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            <button disabled={!summary.trim() || !picked[selected]?.length}>
              Commit {picked[selected]?.length ?? 0} file(s)
            </button>
          </form>
        )}
        {preview && (
          <dialog
            ref={(d) => {
              if (d && !d.open) d.showModal();
            }}
            onClose={() => setPreview(null)}
          >
            <pre>{preview.text}</pre>
            <button onClick={preview.confirm}>Confirm</button>
            <button onClick={() => setPreview(null)}>Cancel</button>
          </dialog>
        )}
      </main>
    </div>
  );
}

export default App;
