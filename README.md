# Ai Git Commit

Desktop git GUI (Tauri 2 + React + TypeScript). The Rust backend shells out to your system `git`, so your existing config, SSH keys and credential helpers apply.

## Prerequisites

- [Rust](https://rustup.rs) (stable)
- [Node.js](https://nodejs.org) 20+
- `git` on your PATH (`xcode-select --install` or `brew install git`)

## Develop

```sh
npm install
npm run tauri dev
```

Rust tests: `cd src-tauri && cargo test`.

## Build (macOS)

```sh
npm run tauri build -- --bundles app,dmg
```

Output (unsigned):

- `src-tauri/target/release/bundle/macos/Ai Git Commit.app`
- `src-tauri/target/release/bundle/dmg/Ai Git Commit_<version>_aarch64.dmg`

The app is unsigned, so on first launch right-click it → **Open**, or run `xattr -dr com.apple.quarantine "/Applications/Ai Git Commit.app"`.

## Data

Stored in `~/Library/Application Support/com.tinkeshwar.gittool/`:

- `repos.json`: the repository list
- `ai.json`: AI provider settings, including API keys (plain text)

## PATH when launched from Finder

macOS gives Finder-launched apps a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), so Homebrew `git`, `git-lfs`, `gh` credential helpers and the AI CLIs would not be found. On startup the app adopts your login shell's PATH (`$SHELL -lc 'echo $PATH'`). If a tool still isn't found, add its directory to PATH in `~/.zprofile` (login shells don't read `~/.zshrc`).

SSH works without extra setup: launchd gives GUI apps `SSH_AUTH_SOCK`, and keys in the macOS keychain (`UseKeychain yes` in `~/.ssh/config`) load automatically. HTTPS credentials go through git's `credential.helper` (e.g. `osxkeychain`). The app sets `GIT_TERMINAL_PROMPT=0`, so a missing credential fails with an error instead of hanging.
