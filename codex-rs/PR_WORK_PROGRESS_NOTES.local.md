## PR Work Progress Notes (local only)

Do not commit this file. Running log for preparing the sessions picker/CLI PR.

Branch: feature/sessions-picker-cli
Base: origin/main (5eaaf307)
Date: 2025‑09‑09

Done
- Rebased branch onto origin/main without opening editors.
- Resolved conflicts in core/config.rs, core/rollout/recorder.rs, prompt_caching tests, and one TUI snapshot.
- Stabilized TUI transcript banner (lowercase "codex"; no version).
- Fixed sessions picker imports/types (use rollout::recorder helpers, convert ConversationId→Uuid).
- codex-cli sessions list/name wired to recorder helpers.
- Tests: core 188/188, tui 237/237 (1 ignored), full workspace all‑features green.

Next (optional)
- History rewrite to purge accidental tracking of notes files.
- Picker quality: fuzzy filter, pin/unpin, quick actions (open/copy), gc tools.

Notes
- Keep WORK.md and this file untracked; never use `git add .`. Prefer `git add -u` or `git add -p`.
