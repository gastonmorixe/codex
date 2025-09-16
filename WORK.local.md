# WORK.md

Lightweight working log and TODOs for this session (United States, 2025-08-28).

## Active Goals (2025-09-09)
- [x] Rebase feature/sessions-picker-cli onto origin/main without opening editors.
- [x] Resolve conflicts in core/config.rs, core/rollout/recorder.rs, core tests, tui snapshot.
- [x] Make TUI banner stable for snapshots; fix sessions picker imports and types.
- [x] Get cargo test -p codex-core, -p codex-tui to green; then cargo test --all-features.
- [x] Keep WORK.md and PR_WORK_PROGRESS_NOTES.md untracked.

## Ground Rules
- Prefer minimal, targeted diffs; avoid unrelated changes.
- For `codex-rs`:
  - Run `just fmt` before finalizing.
  - Run `just fix -p <project>` to fix lints.
  - Run tests for touched crates first (e.g., `cargo test -p codex-tui`), then run the full suite if common/core/protocol changed (`cargo test --all-features`).
  - When running interactively, ask before executing these commands.
  - TUI styling: see `codex-rs/tui/styles.md`; use ratatui `Stylize` helpers (e.g., `"text".red()`).
  - Snapshot tests: use `cargo insta` workflow as documented in repo.
- Never add or modify any code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR`.

## Decisions
- Keep local notes (WORK.md, codex-rs/PR_WORK_PROGRESS_NOTES.md) untracked at all times. Never add via blanket add; prefer `git add -u` or `git add -p`.
- Do not touch CODEX_SANDBOX_* env var logic per repo rules.

## Notes
- Rebase summary: onto origin/main (5eaaf307). Conflicts: core/config.rs (added auto_session_title, disable_response_storage; updated tests), core/rollout/recorder.rs (session meta includes cwd; writer spawn unified), core prompt_caching tests (structural env assertions), tui snapshot (line wrapping).
- TUI changes: history banner now "codex" without version to stabilize snapshots; sessions picker imports fixed to use rollout::recorder helpers; UUID conversions applied.
- CLI: sessions list/name imports corrected; id uses `Uuid` converted from `ConversationId`.

## TODOs
- [ ] Optional: history rewrite to purge accidental tracking of notes files.
- [ ] Optional: add fuzzy filter to sessions picker.

## Follow-ups
- [ ] 

—
Tip: When using `format!`, inline variables directly with `{}`.
