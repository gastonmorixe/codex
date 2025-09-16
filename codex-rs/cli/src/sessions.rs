//! CLI for inspecting and managing recorded sessions (rollouts).
//!
//! Examples:
//! - `codex sessions list -n 25 --json`
//! - `codex sessions name 1a2b3c4d "Bug triage"`
//! - `codex sessions name ~/.codex/sessions/2025/08/28/rollout-....jsonl "Hotfix"`

use clap::Parser;
use clap::Subcommand;
use codex_common::CliConfigOverrides;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::rollout::recorder::SessionStateSnapshot;
use codex_core::rollout::recorder::append_state_line;
use codex_core::rollout::recorder::read_session_header_and_state;
use std::path::Path;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Parser)]
pub struct SessionsCli {
    #[clap(subcommand)]
    cmd: SessionsSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SessionsSubcommand {
    /// List recent sessions (newest first).
    List {
        /// Maximum number of sessions to show (default 10).
        #[arg(long = "limit", short = 'n')]
        limit: Option<usize>,
        /// Output as JSON (array of objects).
        #[arg(long = "json", default_value_t = false)]
        json: bool,
    },

    /// Assign or update a human-friendly name for a session.
    Name {
        /// Session id (full or 8-char prefix) or a path to the .jsonl file.
        id_or_path: String,
        /// The name to assign.
        name: String,
    },
}

pub async fn run_sessions_main(cli: SessionsCli) -> anyhow::Result<()> {
    match cli.cmd {
        SessionsSubcommand::List { limit, json } => list_cmd(limit.unwrap_or(10), json).await,
        SessionsSubcommand::Name { id_or_path, name } => name_cmd(&id_or_path, &name).await,
    }
}

async fn load_config() -> anyhow::Result<Config> {
    // Mirror other subcommands: accept root-level -c overrides if needed later.
    let overrides_cli = CliConfigOverrides::default();
    let kv = overrides_cli.parse_overrides().unwrap_or_default();
    let cfg = Config::load_with_cli_overrides(kv, ConfigOverrides::default())?;
    Ok(cfg)
}

async fn list_cmd(limit: usize, json: bool) -> anyhow::Result<()> {
    let cfg = load_config().await?;
    let mut entries = collect_sessions(&cfg.codex_home, limit)?;
    // newest first by timestamp
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries.truncate(limit);

    if json {
        let json_vals: Vec<serde_json::Value> = entries
            .into_iter()
            .map(|e| {
                let working_path = e.cwd.as_ref().map(|p| p.to_string_lossy().to_string());
                serde_json::json!({
                    "id": e.id,
                    "timestamp": e.timestamp,
                    "name": e.state.name,
                    "path": e.path,
                    // Include session working directory even if null for older sessions.
                    "working_path": working_path,
                    // Additional metadata for tooling: last modification seconds since epoch.
                    "last_modified": e.last_modified_epoch,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_vals)?);
    } else {
        for e in entries.into_iter().take(limit) {
            // Primary line: local-ish compact timestamp, 8-char id, and title.
            let id8 = &e.id.as_hyphenated().to_string()[..8];
            let time_compact = compact_time(&e.timestamp);
            let title = session_title(&e);

            println!("{}  {}  {}", time_compact, id8, title);

            // Secondary: path and metadata, dimmed to match TUI conventions.
            let short_path = shorten_path(&e.path);
            let mut meta_parts: Vec<String> = Vec::new();
            if let Some(cwd) = e.cwd.as_ref() {
                meta_parts.push(format!("cwd: {}", shorten_path(cwd)));
            }
            if let Some(ago) = e.last_activity_ago.as_ref() {
                meta_parts.push(format!("last: {}", ago));
            }
            if !meta_parts.is_empty() {
                println!("{}", dim(&format!("    {}", meta_parts.join("  •  "))));
            }
            println!("{}", dim(&format!("    └ {}", short_path)));
        }
    }
    Ok(())
}

struct FullEntry {
    id: uuid::Uuid,
    timestamp: String,
    state: SessionStateSnapshot,
    path: PathBuf,
    cwd: Option<PathBuf>,
    last_activity_ago: Option<String>,
    last_modified_epoch: Option<u64>,
}

fn collect_sessions(root: &Path, _limit: usize) -> std::io::Result<Vec<FullEntry>> {
    let mut out = Vec::new();
    let sessions_root = root.join("sessions");
    if !sessions_root.exists() {
        return Ok(out);
    }
    for entry in WalkDir::new(&sessions_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy();
        if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
            continue;
        }
        if let Ok((meta, state)) = read_session_header_and_state(entry.path()) {
            // File metadata for last activity
            let (ago, epoch) = file_last_activity(entry.path());
            out.push(FullEntry {
                id: meta.id.into(),
                timestamp: meta.timestamp,
                state,
                path: entry.path().to_path_buf(),
                cwd: Some(meta.cwd),
                last_activity_ago: ago,
                last_modified_epoch: epoch,
            });
        }
    }
    Ok(out)
}

async fn name_cmd(id_or_path: &str, name: &str) -> anyhow::Result<()> {
    let cfg = load_config().await?;
    let p = resolve_session_path(&cfg.codex_home, id_or_path)?;
    let state = SessionStateSnapshot {
        name: Some(name.to_string()),
    };
    append_state_line(&p, &state)?;
    println!("named: {}", p.display());
    Ok(())
}

fn resolve_session_path(root: &Path, id_or_path: &str) -> anyhow::Result<PathBuf> {
    let as_path = PathBuf::from(id_or_path);
    if as_path.exists() {
        return Ok(as_path);
    }
    // Treat as id prefix and scan
    let entries = collect_sessions(root, usize::MAX)?;
    let matches: Vec<_> = entries
        .into_iter()
        .filter(|e| e.id.as_simple().to_string().starts_with(id_or_path))
        .collect();
    let mut it = matches.into_iter();
    match (it.next(), it.next()) {
        (None, _) => anyhow::bail!("no session found matching id prefix: {}", id_or_path),
        (Some(one), None) => Ok(one.path),
        (Some(_), Some(_)) => anyhow::bail!(
            "multiple sessions match prefix {}; please be more specific",
            id_or_path
        ),
    }
}

// --- helpers: user-facing formatting (no extra deps) ---

fn compact_time(iso_ts: &str) -> String {
    // Input example: 2025-08-28T17:59:34.062Z
    // Produce: 2025-08-28 17:59 (drop seconds/subsec and 'Z')
    let s = iso_ts.replace('T', " ").replace('Z', "");
    if let Some((date_time, _rest)) = s.split_once('.') {
        // Drop subseconds
        if let Some((date_hm, _sec)) = date_time.rsplit_once(':') {
            // Drop seconds part
            return date_hm.to_string();
        }
        return date_time.to_string();
    }
    if let Some((date_hm, _sec)) = s.rsplit_once(':') {
        return date_hm.to_string();
    }
    s
}

fn file_last_activity(p: &Path) -> (Option<String>, Option<u64>) {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;
    let meta = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return (None, None),
    };
    let mt = match meta.modified() {
        Ok(t) => t,
        Err(_) => return (None, None),
    };
    let epoch = mt.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    let now = SystemTime::now();
    let ago = match now.duration_since(mt) {
        Ok(d) => Some(format!("{} ago", human_duration(d))),
        Err(_) => None,
    };
    (ago, epoch)
}

fn human_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 90 {
        return format!("{}s", s);
    }
    if s < 90 * 60 {
        return format!("{}m", s / 60);
    }
    if s < 48 * 3600 {
        return format!("{}h", s / 3600);
    }
    format!("{}d", s / 86_400)
}

fn shorten_path(p: &Path) -> String {
    use std::env;
    let s = p.to_string_lossy();
    if let Ok(home) = env::var("HOME") {
        if s.starts_with(&home) {
            let rest = &s[home.len()..];
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            return format!("~/{rest}");
        }
    }
    s.into_owned()
}

fn dim(s: &str) -> String {
    // ANSI SGR 2 = dim; 0 = reset
    format!("\x1b[2m{}\x1b[0m", s)
}

fn session_title(e: &FullEntry) -> String {
    if let Some(name) = e.state.name.as_ref() {
        if !name.trim().is_empty() {
            return name.clone();
        }
    }
    // Fall back to first line of instructions if present in the header.
    // We do not have instructions in FullEntry; re-read the header line cheaply.
    if let Ok(text) = std::fs::read_to_string(&e.path) {
        if let Some(first) = text.lines().next() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(first) {
                if let Some(instr) = v.get("instructions").and_then(|v| v.as_str()) {
                    let mut line = instr.lines().next().unwrap_or("").trim().to_string();
                    if line.is_empty() {
                        line = "(no title)".to_string();
                    }
                    if line.len() > 80 {
                        line.truncate(80);
                    }
                    return line;
                }
            }
        }
    }
    "(no title)".to_string()
}
