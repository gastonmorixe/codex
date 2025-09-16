
## 2025-09-15 18:02

Started git maintenance: fetch upstream, rebase on upstream/main, fix conflicts, build.
Steps: check remotes/branch, fetch, rebase, resolve, build.
Rebasing onto origin/main as upstream per user instruction.

- Rebase started; conflicts in cli/Cargo.toml, core/src/codex.rs, tui/src/lib.rs. Inspecting.

## 2025-09-15 19:12

Continuing rebase with editor disabled to avoid hangs.

## 2025-09-15 19:22

Rebase continued and resolved further conflicts by preferring origin/main.
Fixed build issues in TUI: add uuid dep, switch dirs_next->dirs, import GitInfo from protocol, fix SessionStateSnapshot path, set cwd: Some(meta.cwd).
Added missing mod session_title; in tui/src/lib.rs.

## 2025-09-15 19:24

cargo build --workspace succeeded after fixes.

## 2025-09-15 21:19

Investigating how to enable raw reasoning output and other debugging logs.
Plan: search configs/flags affecting reasoning visibility and logging.

## 2025-09-15 21:24

Confirmed raw reasoning controlled by Config.show_raw_agent_reasoning; can set via -c show_raw_agent_reasoning=true or config.toml.
Exec prints reasoning to stdout when enabled; TUI uses same config and logs to ~/.codex/log/codex-tui.log when RUST_LOG set.
Debug logs controlled via RUST_LOG; TUI session JSON log via CODEX_TUI_RECORD_SESSION env.

## 2025-09-16 14:07

Documented additional config toggles: reasoning, tool flags, sandbox extras, session logging env vars.
Captured how to set them via -c overrides or config.toml.

## 2025-09-16 14:08

Flags/config options identified today:
- show_raw_agent_reasoning (bool) — surface raw thinking stream.
- hide_agent_reasoning (bool) — suppress high-level reasoning channel.
- model_reasoning_effort = minimal|low|medium|high.
- model_reasoning_summary = auto|concise|detailed|none.
- model_verbosity = low|medium|high.
- model_supports_reasoning_summaries (bool).
- auto_session_title (bool).
- include_plan_tool (bool).
- include_apply_patch_tool (bool).
- include_view_image_tool (bool).
- tools.web_search / tools_web_search_request (bool).
- use_experimental_streamable_shell_tool (bool).
- use_experimental_unified_exec_tool (bool).
- sandbox_workspace_write.* tweaks (exclude_tmpdir_env_var, exclude_slash_tmp, extra roots).
- disable_paste_burst (bool).
- tui_notifications command (vector).
- RUST_LOG env for log verbosity.
- CODEX_TUI_RECORD_SESSION / CODEX_TUI_SESSION_LOG_PATH envs for session JSONL logging.
- experimental session picker flags: --experimental-list-sessions, --experimental-sessions-limit, --experimental-resume.
