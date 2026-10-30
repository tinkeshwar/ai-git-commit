# Ai Git Commit

A small desktop git client focused on one loop: see what changed, pick files, get an AI-written commit message, commit, push.

Built with Tauri 2 + React + TypeScript. The Rust backend shells out to your system `git`, so your existing config, SSH keys and credential helpers just work.

## Features

- **Repositories**: add a local repo or clone from a URL; the list persists between launches.
- **Status & diff**: changed and untracked files with a per-file diff view.
- **Commit**: commit only the files you pick, with a preview of what will be committed.
- **Fetch / pull / push**: each with a preview (incoming commits, `push --dry-run`) before it runs.
- **AI commit messages**: generates a [Conventional Commits](https://www.conventionalcommits.org) message (summary + body) from the selected files' diff. Lockfiles are skipped and large diffs are truncated (4k chars per file, 16k total).

### AI providers

| Kind | Providers |
|------|-----------|
| HTTP API (key required) | OpenAI, Anthropic, Gemini, Groq, OpenRouter, Mistral, DeepSeek |
| Local (no key) | Ollama, LM Studio |
| Custom | Any OpenAI-compatible base URL |
| CLI (uses your existing login) | `claude`, `gemini`, `codex` |

Pick a provider in **AI Settings**, set the model, and use **Test** to check it. Installed CLIs are detected automatically.

## Install

Download the latest build from [Releases](https://github.com/tinkeshwar/ai-git-commit/releases):

- **macOS**: universal `.dmg` (Apple Silicon + Intel)
- **Windows**: NSIS `.exe` installer

Builds are unsigned. On macOS, right-click the app → **Open** on first launch, or run:

```sh
xattr -dr com.apple.quarantine "/Applications/Ai Git Commit.app"
```

`git` must be on your PATH (`xcode-select --install` or `brew install git` on macOS).

## Develop

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20+, `git`.

```sh
npm install
npm run tauri dev
```

Rust tests:

```sh
cd src-tauri && cargo test
```

### Build locally (macOS)

```sh
npm run tauri build -- --bundles app,dmg
```

Output goes to `src-tauri/target/release/bundle/` (`macos/Ai Git Commit.app` and `dmg/Ai Git Commit_<version>_aarch64.dmg`).

## Releases

Development happens on Forgejo and is mirrored to GitHub.

1. A push to `main` on Forgejo runs `commit-and-tag-version`, which bumps the version, updates [CHANGELOG.md](CHANGELOG.md) and tags `vX.Y.Z` from the Conventional Commit history.
2. The branch and tags are mirrored to GitHub.
3. The `v*` tag triggers the GitHub release workflow, which builds macOS and Windows bundles into a **draft** release. Publish it manually.

Use Conventional Commit messages (`feat:`, `fix:`, …) so the version bump and changelog come out right.

## Data

Stored in `~/Library/Application Support/com.tinkeshwar.gittool/` on macOS (`%APPDATA%\com.tinkeshwar.gittool\` on Windows):

- `repos.json`: the repository list
- `ai.json`: AI provider settings, **including API keys in plain text**

## Notes for macOS

**PATH when launched from Finder.** Finder-launched apps get a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), which would hide Homebrew `git`, `git-lfs`, `gh` credential helpers and the AI CLIs. On startup the app adopts your login shell's PATH (`$SHELL -lc 'echo $PATH'`). If a tool still isn't found, add its directory to PATH in `~/.zprofile` (login shells don't read `~/.zshrc`).

**Authentication.** SSH works without extra setup: launchd provides `SSH_AUTH_SOCK`, and keychain-stored keys (`UseKeychain yes` in `~/.ssh/config`) load automatically. HTTPS goes through git's `credential.helper` (e.g. `osxkeychain`). The app sets `GIT_TERMINAL_PROMPT=0`, so a missing credential fails with an error instead of hanging.
