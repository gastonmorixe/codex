TUI: Sessions Picker + CLI: Sessions Management; Core: Named Sessions

Summary
- Add an interactive sessions picker to Codex TUI (new flags) so users can resume a recent session without hunting for a rollout path.
- Add a `codex sessions` CLI for listing and naming sessions.
- Extend rollout state to support an optional human-friendly session name, and expose helpers to read/write this metadata.
- Update README with usage examples.

Why
Resuming work across restarts is a common workflow. Today, users must locate a JSONL rollout path and pass it via `-c experimental_resume=...`. This PR streamlines that flow with an in-product picker, plus a small CLI for scriptable management.

What’s Included
1) TUI (codex-tui)
- Flags
  - `--experimental-list-sessions`: open a picker of recent sessions (newest first).
  - `--experimental-sessions-limit N`: limit rows (default 10).
  - `--experimental-resume [FILE]`: resume directly when a FILE is given; with no value, show the picker.
- Picker behavior and keys
  - Up/Down: move; Enter: resume; Esc: cancel.
  - Rows show: local timestamp → 8-char id prefix → title (session name or first line of instructions). The rollout path is shown dimmed.
- Implementation notes
  - `tui/src/sessions_picker.rs` (new): reads `~/.codex/sessions/**/rollout-*.jsonl`, parses header + latest state, sorts by timestamp desc, renders via existing shared popup/table utilities.
  - Reuses the TUI’s selection popup styling for a consistent look.

2) CLI Multitool (codex)
- `codex sessions list [-n N] [--json]`: newest-first listing; prints id, timestamp, name, and path.
- `codex sessions name <id-or-path> "Name"`: set/overwrite a friendly name using a rollout path or UUID prefix (first 8 chars).
- Implementation: `cli/src/sessions.rs` + `walkdir` dependency.

3) Core (codex-core)
- `rollout::SessionStateSnapshot` now includes an optional `name: Option<String>`.
- New helpers:
  - `read_session_header_and_state(path) -> (SessionMeta, SessionStateSnapshot)`
  - `append_state_line(path, &SessionStateSnapshot)` appends a `record_type: "state"` line; no header rewrite.
- `codex::record_state_snapshot` updated to construct the new struct.
- `rollout` module exported publicly for TUI/CLI access.

4) Docs
- `README.md`: new “Sessions: resume, pick, and name” section with examples (TUI flags, CLI usage).

Usage Examples
- TUI picker: `codex --experimental-list-sessions`
- Picker with limit: `codex --experimental-list-sessions --experimental-sessions-limit 25`
- Prompted resume (picker): `codex --experimental-resume`
- Direct resume: `codex --experimental-resume /abs/path/to/rollout-...jsonl`
- CLI list: `codex sessions list -n 25 --json`
- CLI name: `codex sessions name 1a2b3c4d "Hotfix rollout"`

Compatibility & Notes
- No breaking changes to existing flags; `-c experimental_resume=...` still works.
- Rollout files remain append-only; naming is recorded as a subsequent `state` line.
- Picker uses absolute paths as-is; relative paths are resolved against the configured cwd.

Tests & Quality
- `cargo fmt` run.
- TUI: 170 tests passed.
- Core: 178 tests passed (+ 35 integration tests, 2 ignored) and full workspace suite green.
- macOS seatbelt warnings in some integration tests remain expected; unrelated to this change.

Future Work (follow-ups welcome)
- Filter in picker (type-to-filter with fuzzy match).
- Pin/unpin sessions (pins sort first).
- Open in editor from picker (`o`), copy path (`y`).
- Delete/gc: `codex sessions rm`, retention policies / dry-run.
- Preview pane: show the first N messages alongside the list.
- Export transcript to Markdown from CLI.
- Config toggle to always open picker at startup.

Screenshots
- N/A in this PR; the picker reuses existing popup styling.

Thank you!
