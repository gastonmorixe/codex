# Security fix: MacOS secure login with native browser companion app

Introduction
On macOS, localhost-based login is fragile: modern browsers and network controls often block or challenge https→http redirects to localhost, while corporate proxies and captive portals can break callback flows. Running a local HTTP server adds friction (ports, firewall prompts, timeouts). This PR replaces that fragile step on macOS with a tiny native helper app (WKWebView). The helper opens the authorize URL, intercepts the localhost callback purely inside WebKit, and returns only the OAuth code/state to the CLI. The CLI performs PKCE state validation, token exchange, and secure persistence — no local server or external browser redirect needed. This hardens security, improves reliability, and streamlines the user experience.

Summary
- Adds a macOS-native browser companion app (WKWebView) to complete ChatGPT login without a local HTTP server.
- Intercepts the localhost callback inside the helper; only returns code/state to the CLI for token exchange and persistence.
- PKCE + state validation is enforced by the CLI; auth.json persisted with restrictive permissions.

Key Changes
- New CLI path: `codex login --browser` (macOS only)
  - Swift helper opens the authorize URL; intercepts `http://localhost/.../auth/callback` in WKWebView.
  - Emits `{"code","state"}` JSON to stdout; exits. The CLI exchanges tokens and persists credentials.
  - Menus enabled (About/Quit, Cut/Copy/Paste/Select All). Copy/paste works in form fields.
  - Closing the window cleanly aborts login (exit code 2). CLI prints a neutral “Login aborted …” message.
- CLI UX/security
  - Re-login prompt now only when a persisted `auth.json` exists. An env-only `OPENAI_API_KEY` no longer triggers “already logged in”.
  - Success includes a ✅ plus basic details (email/plan) when available; errors are clear and typed.
- Typed errors
  - New `LoginError` enum: `Aborted`, `UnsupportedOs`, `StateMismatch`, `InvalidHelperResponse`, `TokenExchangeFailed`, `Network`, `Io`, etc., with clear CLI handling.
- Build (macOS)
  - Build-time embedding: `login/build.rs` now compiles a universal (arm64 + x86_64) helper and embeds it into the crate on macOS only.
  - Fallback: If build-time embedding fails (no swiftc SDK), runtime can compile on-demand the first time `codex login --browser` runs (requires `swiftc`).
  - Non‑macOS targets do not compile or embed the helper.
- MCP reporting
  - `getAuthStatus`/`logout` reflect persisted auth (`auth.json`) rather than ambient env-only keys (clear “logged in” semantics for tools/clients).

Documentation
- `docs/macos-native-browser-login.md`
  - Usage, requirements (Xcode CLT / `swiftc`), universal helper build, runtime fallback.
  - Behavior, troubleshooting, security notes, and high-level design.
- `docs/release_management.md`
  - Platform-specific notes: macOS builds embed a universal helper; Linux/Windows do not include it.

CI
- `.github/workflows/rust-ci.yml`
  - Added a macOS-only job that runs the login crate test suite on macos-14.
  - Existing matrix unchanged; we can make the new job required if desired.

Tests
- Login crate (macOS-only integration tests):
  - Success: persists tokens + API key via local `tiny_http` issuer stub.
  - Abort: helper abort yields `LoginError::Aborted`.
  - State mismatch: forced state vs helper mismatch yields `LoginError::StateMismatch`.
  - Token exchange failure: 500 from issuer yields `LoginError::TokenExchangeFailed`.
  - Unix file permissions: `auth.json` written with `0600` (no group/other perms).
- Inline unit tests:
  - authorize URL has all required params in `native_browser.rs`.
  - nested claims parsing from JWT works.
  - Non-mac path returns `UnsupportedOs`.
- Determinism for mac tests: debug-only env seams (issuer/state/helper-json) used; tests serialize env to avoid races.

Security Considerations
- Helper uses a non-persistent `WKWebsiteDataStore`.
- Helper returns only code/state; tokens do not leave the helper.
- Strict state validation; explicit abort handling.
- `auth.json` stored with restrictive permissions on Unix.

Build & Distribution
- macOS release builds embed a universal helper (arm64 + x86_64). Linux/Windows exclude the helper entirely.
- If `swiftc` is missing at build time, we warn and rely on on-demand compile when the feature is used on a machine with `swiftc`.
- Optional notarization guidance (for zipped releases) available; Homebrew builds from source and does not require signing/notarization.

Backward Compatibility
- Default login (without `--browser`) is unchanged.
- Non-macOS: `codex login --browser` shows a friendly “only supported on macOS” error.
- MCP `getAuthStatus`/`logout` semantics improved to reflect persisted auth only (explicit user opt-in).

How To Verify
- macOS:
  - `cargo build --release`
  - Verify helper: `xcrun lipo -info target/release/build/codex-login-*/out/codex-auth-helper` shows `x86_64 arm64`.
  - Run: `./target/release/codex login --browser`
    - Window appears; completing login persists `auth.json` to `~/.codex/auth.json`. Closing window exits 2 with “Login aborted …”.
- Linux/Windows:
  - `cargo build --release --target <gnu/musl/msvc>`; no helper present; `--browser` prints not supported.

Files of Interest (diff summary)
- `.github/workflows/rust-ci.yml`: add macOS login test job
- `CHANGELOG.md`: add feature/UX/CI notes
- `README.md`: mention/usage addition for `--browser`
- `codex-rs/cli/src/main.rs`: add `--browser` flag wiring
- `codex-rs/cli/src/login.rs`: logic/UX for re-login confirmation; native path handling
- `codex-rs/login/Cargo.toml`: enable `build.rs`
- `codex-rs/login/build.rs`: compile universal helper (per-arch + lipo), fallback, embedding
- `codex-rs/login/src/error.rs`: new typed `LoginError`
- `codex-rs/login/src/lib.rs`: exported APIs; persisted-only helper; auth helpers
- `codex-rs/login/src/native_browser.rs`: native flow implementation, debug-only test seams, early helper bypass
- `codex-rs/login/src/native_browser_helper.swift`: helper source (single-sourced)
- `codex-rs/login/tests/suite/{mod.rs,native_browser.rs}`: macOS integration tests
- `codex-rs/mcp-server/src/codex_message_processor.rs`: persisted-only auth reporting in getAuthStatus/logout
- `docs/macos-native-browser-login.md`: user / dev docs
- `docs/release_management.md`: platform-specific notes

Open Questions / Next Steps
- Do we want to make the macOS-only test job required for PRs?
- Optional: add notarization steps for macOS artifacts in the release workflow for smoother Gatekeeper behavior if we publish zip downloads.
- Future parity: consider Windows/Linux equivalents for a native browser-assisted flow.

Thank you for reviewing!

