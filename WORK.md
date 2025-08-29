# WORK.md

Lightweight working log and TODOs for this session (United States, 2025-08-28).

## Active Goals
- [ ] Clarify immediate task/request.
- [ ] Make small, focused changes with tests.
- [ ] Keep changes consistent with repo style.

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
- 

## Notes
- 

## TODOs
- [ ] 

## Follow-ups
- [ ] 

—
Tip: When using `format!`, inline variables directly with `{}`.
