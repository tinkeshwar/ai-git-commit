import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

type Repo = { path: string; name: string; branch: string | null };

function App() {
  const [repos, setRepos] = useState<Repo[]>([]);
  const [error, setError] = useState<string | null>(null);

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
        <ul>
          {repos.map((r) => (
            <li key={r.path} title={r.path}>
              <span className="name">{r.name}</span>
              <span className="branch">{r.branch ?? "—"}</span>
              <button aria-label={`Remove ${r.name}`} onClick={() => call("remove_repo", { path: r.path })}>
                ×
              </button>
            </li>
          ))}
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
