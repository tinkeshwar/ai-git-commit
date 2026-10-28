# git-tool

Desktop git GUI (Tauri 2 + React + TypeScript). Rust backend shells out to system `git`.
Planning lives in **ai-planner**, project `git-tool`. Main plan: `001-git-gui-tool-v1`.

## Document kinds
- **Plan**: multi-phase design. May have open questions. Never implement directly; split into issues once approved. Create with `type: "plan"`, `status: "draft"`, no `NNN-` prefix in the title (slugs are auto-numbered).
- **Audit**: security/bug/perf/gap review of the app.
- **Issue**: self-contained, ready-to-work unit. `type`: `bug | feature | enhancement | task`. Zero open questions, one problem per issue. Never create a plan when an issue is asked for.
- **Docs**: guides. Never create one without approval.

Audits and issues: set `service` to `git-tool`.

## Workflow
- **By number** ("issue 4", "plan 001"): `get_issue_by_number` / `get_plan_by_number` first; fall back to `list_*` and match the slug prefix (`001-*`).
- **Otherwise**: `list_plans` / `list_audits` / `list_issues` (filter by `service`), then `get_*` by id.
- **Open questions** in any doc: halt and ask via `AskUserQuestion`. Never assume defaults. Record the answers in the doc before continuing.
- **Plan → issues** (after approval, no open questions): at least one issue per phase, the plan slug in each issue body, and the issue list in the plan content. Always add compacted context in the ticket before setting status `fixed`.

## Status lifecycle
Update status in the same turn the transition happens, and append `**Status:** <status> YYYY-MM-DD. <reason>` to the doc body.
- **Plan**: `draft` (drafting/review) → `active` (split into issues, being implemented; tick phase boxes as completed) → `done` (whole plan converted into issues).
- **Issue**: `open` (unstarted/in progress) → `fixed` (code change in, commit not required) → `resolved` (verified by tests or the user). `closed` = won't-fix/obsolete, with rationale.
