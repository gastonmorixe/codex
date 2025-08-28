//! CLI for inspecting and managing recorded sessions (rollouts).
//!
//! Examples:
//! - `codex sessions list -n 25 --json`
//! - `codex sessions name 1a2b3c4d "Bug triage"`
//! - `codex sessions name ~/.codex/sessions/2025/08/28/rollout-....jsonl "Hotfix"`

use clap::{Parser, Subcommand};
use codex_common::CliConfigOverrides;
use codex_core::config::{Config, ConfigOverrides};
use codex_core::rollout::{SessionStateSnapshot, append_state_line, read_session_header_and_state};
use std::path::{Path, PathBuf};
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
                serde_json::json!({
                    "id": e.id,
                    "timestamp": e.timestamp,
                    "name": e.state.name,
                    "path": e.path,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_vals)?);
    } else {
        for e in entries.into_iter().take(limit) {
            let id8 = e.id.as_hyphenated().to_string();
            let id8 = &id8[..8];
            let name = e
                .state
                .name
                .clone()
                .unwrap_or_else(|| "(no name)".to_string());
            println!(
                "{}  {}  {}\n    {}",
                e.timestamp,
                id8,
                name,
                e.path.display()
            );
        }
    }
    Ok(())
}

struct FullEntry {
    id: uuid::Uuid,
    timestamp: String,
    state: SessionStateSnapshot,
    path: PathBuf,
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
            out.push(FullEntry {
                id: meta.id,
                timestamp: meta.timestamp,
                state,
                path: entry.path().to_path_buf(),
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
    match matches.len() {
        0 => anyhow::bail!("no session found matching id prefix: {}", id_or_path),
        1 => Ok(matches.into_iter().next().unwrap().path),
        _ => anyhow::bail!(
            "multiple sessions match prefix {}; please be more specific",
            id_or_path
        ),
    }
}
