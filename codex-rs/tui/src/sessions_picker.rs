use std::path::Path;
use std::path::PathBuf;

// Interactive picker for resuming past sessions.
//
// Keys:
// - Up/Down: move selection
// - Enter: resume the selected session
// - Esc/Ctrl-C: cancel without resuming
//
// Rows show: local timestamp → 8-char id prefix → title (session name or
// first line of instructions). The file path is shown dimmed under each row.

use chrono::DateTime;
use chrono::Utc;
// NaiveDate used by date headers
use chrono::NaiveDate;
use codex_protocol::protocol::GitInfo;
use codex_core::git_info::resolve_root_git_project_for_trust;
use codex_core::rollout::SessionMeta;
use codex_core::rollout::recorder::append_state_line;
use codex_core::rollout::recorder::read_session_header_and_state;
use codex_core::rollout::recorder::SessionStateSnapshot;
use color_eyre::eyre::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use std::collections::HashMap;
// (Widget trait is imported from widgets module for mutable rendering)
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tokio_stream::StreamExt;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::bottom_pane::GenericDisplayRow;
use crate::bottom_pane::ScrollState;
use crate::bottom_pane::render_rows;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;

#[derive(Debug, Clone)]
struct SessionEntry {
    id: Uuid,
    when: DateTime<Utc>,
    title: String,
    path: PathBuf,
    cwd: Option<PathBuf>,
    git: Option<GitInfo>,
    approx_turns: usize,
    duration_secs: Option<u64>,
    #[allow(dead_code)]
    size_bytes: u64,
}

#[derive(serde::Deserialize)]
struct HeaderWithGit {
    #[serde(flatten)]
    meta: SessionMeta,
    #[serde(default)]
    git: Option<GitInfo>,
}

fn read_header(path: &Path) -> std::io::Result<Option<SessionEntry>> {
    // Read header line for meta + git in one pass.
    use std::io::BufRead;
    use std::io::BufReader;
    let f = std::fs::File::open(path)?;
    let mut rdr = BufReader::new(f);
    let mut first = String::new();
    if rdr.read_line(&mut first)? == 0 {
        return Ok(None);
    }
    let header: HeaderWithGit = match serde_json::from_str(&first) {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };
    // Also fetch latest state (for name) using existing helper.
    let (meta, state) = (header.meta, read_session_header_and_state(path)?.1);
    let when = match DateTime::parse_from_rfc3339(&meta.timestamp) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return Ok(None),
    };
    let mut title = state.name.as_deref().unwrap_or("").to_string();
    if title.is_empty() {
        title = meta
            .instructions
            .as_deref()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
    }
    if title.is_empty() {
        title = "(no title)".to_string();
    }
    if title.len() > 80 {
        title.truncate(80);
    }
    let meta_fs = std::fs::metadata(path)?;
    let size_bytes = meta_fs.len();
    let duration_secs = meta_fs
        .modified()
        .ok()
        .and_then(|mt| mt.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|mt| DateTime::<Utc>::from_timestamp(mt.as_secs() as i64, 0))
        .map(|mt| (mt - when).num_seconds().max(0) as u64);

    // Rough turns: number of non-empty lines minus 1 header minus state lines
    // (fast scan; OK if slightly off).
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut non_empty = 0usize;
    let mut state_lines = 0usize;
    for line in reader.lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            non_empty += 1;
            if line.contains("\"record_type\":\"state\"") {
                state_lines += 1;
            }
        }
    }
    let approx_turns = non_empty.saturating_sub(1 + state_lines);
    Ok(Some(SessionEntry {
        id: meta.id.into(),
        when,
        title,
        path: path.to_path_buf(),
        cwd: Some(meta.cwd.clone()),
        git: header.git,
        approx_turns,
        duration_secs,
        size_bytes,
    }))
}

fn collect_recent_sessions(codex_home: &Path, limit: usize) -> std::io::Result<Vec<SessionEntry>> {
    let mut out: Vec<SessionEntry> = Vec::new();
    let root = codex_home.join("sessions");
    if !root.exists() {
        return Ok(out);
    }

    for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        if let Ok(Some(header)) = read_header(entry.path()) {
            out.push(header);
        }
    }
    // newest first
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out.truncate(limit);
    Ok(out)
}

fn shorten_path(p: &Path) -> String {
    let home = dirs::home_dir();
    let s = p.to_string_lossy();
    match (home.as_ref(), &*s) {
        (Some(h), s) => {
            let hp = h.to_string_lossy();
            if s.starts_with(&*hp) {
                let rest = &s[hp.len()..];
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                format!("~/{rest}")
            } else {
                s.to_string()
            }
        }
        _ => s.into_owned(),
    }
}

#[allow(dead_code)]
fn display_title(e: &SessionEntry) -> String {
    e.title.clone()
}

fn build_row_text(e: &SessionEntry) -> (String, String) {
    let id8 = &e.id.as_hyphenated().to_string()[..8];
    let time_str = e.when.format("%Y-%m-%d %H:%M");
    let branch = e.git.as_ref().and_then(|g| g.branch.as_ref());
    let dur = e
        .duration_secs
        .map(|s| {
            if s < 90 {
                format!("{s}s")
            } else if s < 90 * 60 {
                format!("{}m", s / 60)
            } else {
                format!("{}h", s / 3600)
            }
        })
        .unwrap_or_default();
    let turns = if e.approx_turns > 0 {
        format!(" #{}", e.approx_turns)
    } else {
        String::new()
    };
    let badge = match (branch, dur.is_empty()) {
        (Some(b), false) => format!(" [{b}]{turns}  {dur}"),
        (Some(b), true) => format!(" [{b}]{turns}"),
        (None, false) => format!("{turns}  {dur}"),
        (None, true) => turns,
    };
    let primary = format!("{time_str}  {id8}  {}", e.title);

    // Secondary: host + sha7 + shortened path
    let host = e
        .git
        .as_ref()
        .and_then(|g| g.repository_url.as_ref())
        .and_then(|u| url::Url::parse(u).ok())
        .map(|u| u.host_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let sha7 = e
        .git
        .as_ref()
        .and_then(|g| g.commit_hash.as_ref())
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();
    let short_path = shorten_path(&e.path);
    let mut secondary = String::new();
    if !host.is_empty() {
        secondary.push_str(&host);
    }
    if !sha7.is_empty() {
        if !secondary.is_empty() {
            secondary.push(' ');
        }
        secondary.push_str(&sha7);
    }
    if !secondary.is_empty() {
        secondary.push(' ');
    }
    secondary.push_str(&short_path);

    let name = if badge.trim().is_empty() {
        primary
    } else {
        format!("{primary}  {badge}")
    };
    (name, secondary)
}

fn day_header(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        return "Today".to_string();
    }
    if let Some(yesterday) = today.pred_opt()
        && day == yesterday
    {
        return "Yesterday".to_string();
    }
    day.to_string()
}

struct SessionsPickerWidget {
    rows: Vec<GenericDisplayRow>,
    paths: Vec<PathBuf>,
    is_header: Vec<bool>,
    entries: Vec<SessionEntry>,
    state: ScrollState,
    // Number of rows to render and to use for scroll math.
    // Updated dynamically based on the list area's height in `render`.
    visible_rows: usize,
    /// Current working directory of this run; used for mismatch hints.
    current_cwd: PathBuf,

    // Filtering / rename / confirm state
    filtering: bool,
    filter_text: String,
    renaming: bool,
    rename_text: String,
    confirm_resume_path: Option<PathBuf>,
    confirm_delete_path: Option<PathBuf>,
    last_deleted: Option<(PathBuf, PathBuf)>, // (deleted_path, backup_path)
    last_action_hint: Option<String>,
    preview_cache: HashMap<PathBuf, (String, String)>,
}

impl SessionsPickerWidget {
    fn from_entries_with_cwd(entries: &[SessionEntry], current_cwd: &Path) -> Self {
        let mut rows = Vec::new();
        let mut paths = Vec::new();
        let mut is_header = Vec::new();
        for e in entries {
            let cwd_matches = e.cwd.as_ref().map(|c| c == current_cwd).unwrap_or(false);
            let id8 = &e.id.as_hyphenated().to_string()[..8];
            let time_str = e.when.format("%Y-%m-%d %H:%M");
            let branch = e.git.as_ref().and_then(|g| g.branch.as_ref());
            let dur = e
                .duration_secs
                .map(|s| {
                    if s < 90 {
                        format!("{s}s")
                    } else if s < 90 * 60 {
                        format!("{}m", s / 60)
                    } else {
                        format!("{}h", s / 3600)
                    }
                })
                .unwrap_or_default();
            let turns = if e.approx_turns > 0 {
                format!(" #{}", e.approx_turns)
            } else {
                String::new()
            };
            let badge = match (branch, dur.is_empty()) {
                (Some(b), false) => format!(" [{b}]{turns}  {dur}"),
                (Some(b), true) => format!(" [{b}]{turns}"),
                (None, false) => format!("{turns}  {dur}"),
                (None, true) => turns,
            };
            let primary = format!("{time_str}  {id8}  {}", e.title);

            // Secondary line: repo host + short sha + shortened path + cwd + last activity
            let host = e
                .git
                .as_ref()
                .and_then(|g| g.repository_url.as_ref())
                .and_then(|u| url::Url::parse(u).ok())
                .map(|u| u.host_str().unwrap_or("").to_string())
                .unwrap_or_default();
            let sha7 = e
                .git
                .as_ref()
                .and_then(|g| g.commit_hash.as_ref())
                .map(|s| s.chars().take(7).collect::<String>())
                .unwrap_or_default();
            let short_path = shorten_path(&e.path);
            let mut secondary = String::new();
            if !host.is_empty() {
                secondary.push_str(&host);
            }
            if !sha7.is_empty() {
                if !secondary.is_empty() {
                    secondary.push(' ');
                }
                secondary.push_str(&sha7);
            }
            if !secondary.is_empty() {
                secondary.push(' ');
            }
            secondary.push_str(&short_path);
            // Include cwd and last activity if available
            if let Some(cwd) = e.cwd.as_ref() {
                secondary.push_str("  ");
                secondary.push_str(&format!("cwd: {}", shorten_path(cwd)));
            }
            if let Some(ago) = last_activity_ago(&e.path) {
                secondary.push_str("  ");
                secondary.push_str(&format!("last: {ago}"));
            }

            // Compose one logical row; renderer will style parts. Keep secondary in description.
            let name = if badge.trim().is_empty() {
                primary
            } else {
                format!("{primary}  {badge}")
            };
            rows.push(GenericDisplayRow {
                name,
                match_indices: None,
                is_current: cwd_matches,
                description: Some(secondary),
            });
            paths.push(e.path.clone());
            is_header.push(false);
        }
        Self {
            rows,
            paths,
            is_header,
            entries: entries.to_vec(),
            state: ScrollState::new(),
            // Default until first render computes from area height.
            // Start generous; render() will clamp to the available area.
            visible_rows: usize::MAX,
            current_cwd: current_cwd.to_path_buf(),
            filtering: false,
            filter_text: String::new(),
            renaming: false,
            rename_text: String::new(),
            confirm_resume_path: None,
            confirm_delete_path: None,
            last_deleted: None,
            last_action_hint: None,
            preview_cache: HashMap::new(),
        }
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let i = self.state.selected_idx?;
        if *self.is_header.get(i).unwrap_or(&false) {
            return None;
        }
        self.paths.get(i).cloned()
    }

    fn selected_entry_mut(&mut self) -> Option<&mut SessionEntry> {
        let i = self.state.selected_idx?;
        let p = self.paths.get(i)?;
        self.entries.iter_mut().find(|e| &e.path == p)
    }

    fn rebuild_from_filter(&mut self) {
        use chrono::Local;
        self.rows.clear();
        self.paths.clear();
        self.is_header.clear();

        let ft_raw = self.filter_text.trim();
        let ft_lower = ft_raw.to_lowercase();

        // Prepare free-text part for fuzzy highlight
        let qtext = ft_raw
            .split_whitespace()
            .filter(|t| !t.contains(':'))
            .collect::<Vec<&str>>()
            .join(" ");

        // Filter entries
        let mut filtered: Vec<(&SessionEntry, Option<Vec<usize>>)> = Vec::new();
        for e in &self.entries {
            if self.filtering && !ft_lower.is_empty() {
                let branch = e
                    .git
                    .as_ref()
                    .and_then(|g| g.branch.as_ref())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let host = e
                    .git
                    .as_ref()
                    .and_then(|g| g.repository_url.as_ref())
                    .and_then(|u| url::Url::parse(u).ok())
                    .map(|u| u.host_str().unwrap_or("").to_string().to_lowercase())
                    .unwrap_or_default();
                let hay = format!(
                    "{}\n{}\n{}\n{}",
                    e.title.to_lowercase(),
                    branch,
                    host,
                    e.path.to_string_lossy().to_lowercase()
                );
                if !hay.contains(&ft_lower) {
                    continue;
                }
            }
            // fuzzy indices for the title only
            let indices = if !qtext.is_empty() {
                if let Some((idxs, _score)) =
                    codex_common::fuzzy_match::fuzzy_match(&e.title, &qtext)
                {
                    Some(idxs)
                } else {
                    None
                }
            } else {
                None
            };
            filtered.push((e, indices));
        }

        // Group by date with non-selectable headers
        let mut last_date: Option<chrono::NaiveDate> = None;
        for (e, match_indices) in filtered {
            let when_local = e.when.with_timezone(&Local);
            let day = when_local.date_naive();
            if last_date != Some(day) {
                let header = day_header(day, Local::now().date_naive());
                self.rows.push(GenericDisplayRow {
                    name: header.clone(),
                    match_indices: None,
                    is_current: false,
                    description: None,
                });
                self.paths.push(PathBuf::new());
                self.is_header.push(true);
                last_date = Some(day);
            }

            let (primary, secondary) = build_row_text(e);
            self.rows.push(GenericDisplayRow {
                name: primary,
                match_indices,
                is_current: false,
                description: Some(secondary),
            });
            self.paths.push(e.path.clone());
            self.is_header.push(false);
        }
        self.state.clamp_selection(self.rows.len());
        self.state.ensure_visible(
            self.rows.len(),
            self.visible_rows.min(self.rows.len().max(1)),
        );
    }
}

impl Widget for &mut SessionsPickerWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Layout: header, list, footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(3),
                    Constraint::Min(3),
                    Constraint::Length(2),
                ]
                .as_ref(),
            )
            .split(area);

        // Compute dynamic visible rows based on available height, capped to MAX_POPUP_ROWS.
        // Only re-clamp visibility when the window size actually changes to
        // avoid extra one-line scroll adjustments on every draw.
        let area_rows = (chunks[1].height as usize).max(1);
        // Use all available rows for this picker (no 8-row cap).
        let new_visible = area_rows;
        if new_visible != self.visible_rows {
            self.visible_rows = new_visible;
            let total = self.rows.len();
            self.state
                .ensure_visible(total, self.visible_rows.min(total.max(1)));
        }

        // Title with context: filtering/renaming status and counts
        let total = self.rows.len();
        let showing = self.visible_rows.min(total);
        let mut title_parts = vec![
            "Resume a Previous Session".bold().cyan(),
            " — newest first  ".into(),
            format!("showing {showing} of {total}").dim(),
        ];
        if self.filtering {
            title_parts.push("  filter: ".dim());
            title_parts.push(self.filter_text.clone().into());
        }
        if self.renaming {
            title_parts.push("  rename: ".dim());
            title_parts.push(self.rename_text.clone().into());
        }
        let title = Paragraph::new(Line::from(title_parts))
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::NONE)
                    .border_type(BorderType::Plain),
            );
        title.render(chunks[0], buf);

        // List area (uses shared rows renderer for consistent styling)
        render_rows(
            chunks[1],
            buf,
            &self.rows,
            &self.state,
            self.visible_rows,
            false,
            "No sessions found",
        );

        let hint = Paragraph::new(preview_lines(self))
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::NONE));
        hint.render(chunks[2], buf);
    }
}

fn preview_lines(w: &SessionsPickerWidget) -> Vec<Line<'static>> {
    // Show contextual hint for the selected session or active modes.
    let mut out: Vec<Line<'static>> = Vec::new();
    if w.renaming {
        out.push(Line::from("Enter to save, Esc to cancel".dim()));
        return out;
    }
    if let Some(msg) = w.last_action_hint.as_ref() {
        out.push(Line::from(msg.clone().dim()));
        return out;
    }
    if w.filtering {
        out.push(Line::from(
            "Type to filter; Enter keeps filter; Esc clears".dim(),
        ));
        return out;
    }
    if let Some(p) = w.confirm_delete_path.as_ref() {
        out.push(Line::from(
            format!("Delete session file? {}", shorten_path(p)).red(),
        ));
        out.push(Line::from(
            "Enter to delete • Esc to cancel • 'u' to undo last".dim(),
        ));
        return out;
    }
    if let Some(p) = w.confirm_resume_path.as_ref() {
        // Explicit confirmation after a path mismatch. Show recorded vs current directories.
        let recorded = read_header(p)
            .ok()
            .flatten()
            .and_then(|e| e.cwd)
            .map(|c| shorten_path(&c))
            .unwrap_or_else(|| "n/a".to_string());
        out.push(Line::from(
            "⚠ you are resuming a session that was not initiated in the same path ⚠".red(),
        ));
        out.push(Line::from(format!("Recorded path: {recorded}").red()));
        out.push(Line::from(
            format!("Current path: {}", shorten_path(&w.current_cwd)).red(),
        ));
        out.push(Line::from(
            "Press Enter again to resume • Esc to cancel".dim(),
        ));
        return out;
    }
    if let Some(sel) = w.state.selected_idx
        && !w.is_header.get(sel).copied().unwrap_or(false)
        && let Some(row) = w.paths.get(sel)
    {
        // Recover the session entry info by re-reading just the header.
        if let Ok(Some(entry)) = read_header(row) {
            // First line: cwd match/mismatch indicator
            if let Some(rec_cwd) = entry.cwd.as_ref() {
                if rec_cwd == &w.current_cwd {
                    let msg = format!("✓ same path: {} — Enter to resume", shorten_path(row));
                    out.push(Line::from(msg.green()));
                } else {
                    let msg = format!(
                        "⚠ cwd differs: recorded {} vs current {}",
                        shorten_path(rec_cwd),
                        shorten_path(&w.current_cwd)
                    );
                    out.push(Line::from(msg.red()));
                }
            } else {
                out.push(Line::from("⚠ no recorded path (older session)".red()));
            }

            // Second line: repo/branch/host and quick snippets
            let mut parts: Vec<ratatui::text::Span<'static>> = Vec::new();
            if let Some(g) = entry.git.as_ref() {
                if let Some(branch) = g.branch.as_ref() {
                    parts.push(format!("[{branch}] ").dim());
                }
                if let Some(url) = g.repository_url.as_ref()
                    && let Ok(u) = url::Url::parse(url)
                    && let Some(host) = u.host_str()
                {
                    parts.push(host.to_string().dim());
                    parts.push("  ".dim());
                }
            }
            if let Some(entry_host) = entry
                .git
                .as_ref()
                .and_then(|g| g.repository_url.as_ref())
                .and_then(|u| repo_host_from_url(u))
                && let Some(cur_host) = current_repo_host(&w.current_cwd)
                && entry_host != cur_host
            {
                parts.push(format!("⚠ repo differs: {entry_host} vs {cur_host}").red());
                parts.push("  ".into());
            }
            if let Some((first, last)) = w
                .preview_cache
                .get(row)
                .cloned()
                .or_else(|| read_snippets(row))
            {
                parts.push(format!("First: {first}").dim());
                parts.push("  ".dim());
                parts.push(format!("Last: {last}").dim());
            }
            if !parts.is_empty() {
                out.push(Line::from(parts));
            }
        }
    }
    if out.is_empty() {
        out.push(Line::from(vec![
            "↑/↓".into(),
            " move  ".dim(),
            "Enter".into(),
            " resume  ".dim(),
            "Esc".into(),
            " cancel".dim(),
        ]));
    }
    out
}

fn read_snippets(path: &Path) -> Option<(String, String)> {
    use std::io::BufRead;
    use std::io::BufReader;
    let f = std::fs::File::open(path).ok()?;
    let rdr = BufReader::new(f);
    let mut first_user: Option<String> = None;
    let mut last_assistant: Option<String> = None;
    for line in rdr.lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('{')
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && v.get("type").and_then(|t| t.as_str()) == Some("message")
        {
            let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let text = v
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find_map(|item| item.get("text").and_then(|t| t.as_str()))
                })
                .unwrap_or("");
            let snippet = truncate_snippet(text);
            if role == "user" && first_user.is_none() {
                first_user = Some(snippet.clone());
            }
            if role == "assistant" {
                last_assistant = Some(snippet);
            }
        }
    }
    Some((
        first_user.unwrap_or_default(),
        last_assistant.unwrap_or_default(),
    ))
}

fn truncate_snippet(s: &str) -> String {
    let s = s.trim();
    const MAX: usize = 60;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    s.chars().take(MAX - 1).collect::<String>() + "…"
}

fn repo_host_from_url(u: &str) -> Option<String> {
    if let Ok(url) = url::Url::parse(u) {
        return url.host_str().map(|s| s.to_string());
    }
    // SSH style: git@github.com:org/repo.git
    if let Some(pos) = u.find(':') {
        let left = &u[..pos];
        if let Some(at) = left.rfind('@') {
            return Some(left[at + 1..].to_string());
        }
    }
    None
}

fn last_activity_ago(p: &Path) -> Option<String> {
    use std::time::SystemTime;
    let meta = std::fs::metadata(p).ok()?;
    let mt = meta.modified().ok()?;
    let now = SystemTime::now();
    let dur = now.duration_since(mt).ok()?;
    Some(human_ago(dur))
}

fn human_ago(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 90 {
        return format!("{s}s ago");
    }
    if s < 90 * 60 {
        return format!("{}m ago", s / 60);
    }
    if s < 48 * 3600 {
        return format!("{}h ago", s / 3600);
    }
    format!("{}d ago", s / 86_400)
}

fn current_repo_host(cwd: &Path) -> Option<String> {
    let root = resolve_root_git_project_for_trust(cwd)?;
    // Try .git/config
    let cfg = root.join(".git").join("config");
    let path = if cfg.exists() {
        cfg
    } else {
        root.join(".git/config")
    };
    let text = std::fs::read_to_string(path).ok()?;
    // Prefer origin
    let mut best: Option<String> = None;
    let mut in_origin = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_origin = l.contains("remote \"origin\"");
            continue;
        }
        if let Some(rest) = l.strip_prefix("url =") {
            let url = rest.trim();
            let host = repo_host_from_url(url);
            if in_origin {
                return host;
            }
            if best.is_none() {
                best = host;
            }
        }
    }
    best
}

pub(crate) async fn run_sessions_picker_app(
    tui: &mut Tui,
    codex_home: &Path,
    limit: usize,
    current_cwd: &Path,
) -> Result<Option<PathBuf>> {
    // Load entries from disk
    let entries = collect_recent_sessions(codex_home, limit).unwrap_or_default();

    // If nothing to show, return early
    if entries.is_empty() {
        return Ok(None);
    }

    let request_frame: FrameRequester = tui.frame_requester();
    let mut widget = SessionsPickerWidget::from_entries_with_cwd(&entries, current_cwd);
    // Prefer auto-selecting the first row that matches the current cwd.
    if let Some(idx) = widget.rows.iter().position(|r| r.is_current).or(Some(0)) {
        widget.state.selected_idx = Some(idx);
    }
    widget.state.clamp_selection(widget.rows.len());
    widget
        .state
        .ensure_visible(widget.rows.len(), widget.rows.len());

    // Initial draw
    tui.draw(u16::MAX, |frame| {
        let area = frame.area();
        Clear.render(area, frame.buffer_mut());
        frame.render_widget(&mut widget, area);
    })?;

    let mut selected: Option<PathBuf> = None;
    let mut done = false;
    let events = tui.event_stream();
    tokio::pin!(events);

    while !done {
        if let Some(ev) = events.next().await {
            match ev {
                TuiEvent::Key(key) => {
                    use crossterm::event::KeyCode;
                    use crossterm::event::KeyEventKind;
                    use crossterm::event::KeyModifiers;
                    match (key.kind, key.code, key.modifiers) {
                        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Up, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            loop {
                                widget.state.move_up_wrap(widget.rows.len());
                                let i = widget.state.selected_idx.unwrap_or(0);
                                if !widget.is_header.get(i).copied().unwrap_or(false) {
                                    break;
                                }
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Down, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            loop {
                                widget.state.move_down_wrap(widget.rows.len());
                                let i = widget.state.selected_idx.unwrap_or(0);
                                if !widget.is_header.get(i).copied().unwrap_or(false) {
                                    break;
                                }
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::PageUp, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            let step = widget.visible_rows.max(1);
                            for _ in 0..step {
                                widget.state.move_up_wrap(widget.rows.len());
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::PageDown, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            let step = widget.visible_rows.max(1);
                            for _ in 0..step {
                                widget.state.move_down_wrap(widget.rows.len());
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press, KeyCode::Home, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            widget.state.selected_idx = if widget.rows.is_empty() {
                                None
                            } else {
                                Some(0)
                            };
                            if let Some(mut i) = widget.state.selected_idx {
                                while widget.is_header.get(i).copied().unwrap_or(false)
                                    && i + 1 < widget.rows.len()
                                {
                                    i += 1;
                                }
                                widget.state.selected_idx = Some(i);
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press, KeyCode::End, _) => {
                            if widget.renaming || widget.filtering {
                                break;
                            }
                            let len = widget.rows.len();
                            if len == 0 {
                                widget.state.selected_idx = None;
                            } else {
                                let mut i = len - 1;
                                while i > 0 && widget.is_header.get(i).copied().unwrap_or(false) {
                                    i -= 1;
                                }
                                widget.state.selected_idx = Some(i);
                            }
                            widget
                                .state
                                .ensure_visible(widget.rows.len(), widget.visible_rows);
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press, KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            if widget.renaming {
                                widget.renaming = false;
                                widget.rename_text.clear();
                            } else if widget.filtering {
                                widget.filtering = false;
                                widget.filter_text.clear();
                                widget.rebuild_from_filter();
                            } else if widget.confirm_resume_path.is_some() {
                                widget.confirm_resume_path = None;
                            } else {
                                selected = None;
                                done = true;
                            }
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press, KeyCode::Char('/'), _) => {
                            if widget.renaming {
                                break;
                            }
                            widget.filtering = true;
                            widget.filter_text.clear();
                            request_frame.schedule_frame();
                        }
                        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Backspace, _) => {
                            if widget.filtering {
                                let _ = widget.filter_text.pop();
                                widget.rebuild_from_filter();
                                request_frame.schedule_frame();
                            } else if widget.renaming {
                                let _ = widget.rename_text.pop();
                                request_frame.schedule_frame();
                            }
                        }
                        (KeyEventKind::Press, KeyCode::Char(c), _) => {
                            if widget.filtering {
                                if !c.is_control() {
                                    widget.filter_text.push(c);
                                    widget.rebuild_from_filter();
                                    request_frame.schedule_frame();
                                }
                            } else if widget.renaming {
                                if !c.is_control() {
                                    widget.rename_text.push(c);
                                    request_frame.schedule_frame();
                                }
                            } else if c == 'r' {
                                // start rename
                                widget.renaming = true;
                                widget.rename_text = String::new();
                                request_frame.schedule_frame();
                            } else if c == 'd' {
                                if let Some(p) = widget.selected_path() {
                                    widget.confirm_delete_path = Some(p);
                                    request_frame.schedule_frame();
                                }
                            } else if c == 'u' {
                                if let Some((del, backup)) = widget.last_deleted.take() {
                                    let _ = std::fs::rename(&backup, &del);
                                    // refresh list
                                    if let Ok(Some(e)) = read_header(&del) {
                                        if let Some(existing) =
                                            widget.entries.iter_mut().find(|x| x.path == del)
                                        {
                                            *existing = e;
                                        } else {
                                            widget.entries.push(e);
                                        }
                                    }
                                    widget.rebuild_from_filter();
                                    widget.last_action_hint =
                                        Some("Undo: restored deleted session".to_string());
                                    request_frame.schedule_frame();
                                }
                            } else if c == 'y' {
                                if let Some(p) = widget.selected_path() {
                                    widget.last_action_hint =
                                        Some(format!("Path copied: {}", shorten_path(&p)));
                                    request_frame.schedule_frame();
                                }
                            } else if c == 'c' {
                                if let Some(e) = widget.selected_entry_mut() {
                                    widget.last_action_hint = Some(format!("ID copied: {}", e.id));
                                    request_frame.schedule_frame();
                                }
                            } else if c == 'i'
                                && let Some(e) = widget.selected_entry_mut()
                            {
                                let id8 = &e.id.as_hyphenated().to_string()[..8];
                                widget.last_action_hint = Some(format!("id8 copied: {id8}"));
                                request_frame.schedule_frame();
                            }
                        }
                        (KeyEventKind::Press, KeyCode::Enter, _) => {
                            // rename commit, filter accept, or resume (with mismatch confirm)
                            if widget.renaming {
                                let new_name = widget.rename_text.clone();
                                if let Some(entry) = widget.selected_entry_mut()
                                    && !new_name.is_empty()
                                {
                                    let _ = append_state_line(
                                        &entry.path,
                                        &SessionStateSnapshot { name: Some(new_name.clone()) },
                                    );
                                    entry.title = new_name;
                                    widget.rebuild_from_filter();
                                }
                                widget.renaming = false;
                                widget.rename_text.clear();
                                request_frame.schedule_frame();
                                break;
                            }
                            if widget.filtering {
                                widget.filtering = false; // keep filtered results
                                request_frame.schedule_frame();
                                break;
                            }
                            if let Some(sel_path) = widget.confirm_delete_path.take() {
                                // delete confirmed
                                let mut backup = sel_path.clone();
                                backup.set_extension("deleted");
                                let mut n = 0;
                                while backup.exists() {
                                    n += 1;
                                    backup.set_extension(format!("deleted.{n}"));
                                }
                                if std::fs::rename(&sel_path, &backup).is_ok() {
                                    widget.last_deleted = Some((sel_path.clone(), backup));
                                    widget.entries.retain(|e| e.path != sel_path);
                                    widget.rebuild_from_filter();
                                }
                                request_frame.schedule_frame();
                                break;
                            } else if let Some(sel_path) = widget.selected_path() {
                                // If confirmation is pending or no mismatch, proceed; otherwise prompt
                                let needs_confirm = if let Some(entry) =
                                    widget.entries.iter().find(|e| e.path == sel_path)
                                {
                                    if let Some(rec) = entry.cwd.as_ref() {
                                        rec != &widget.current_cwd
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if widget.confirm_resume_path.as_ref() == Some(&sel_path)
                                    || !needs_confirm
                                {
                                    selected = Some(sel_path);
                                    done = true;
                                } else {
                                    widget.confirm_resume_path = Some(sel_path);
                                    request_frame.schedule_frame();
                                }
                            }
                        }
                        (KeyEventKind::Press, KeyCode::Esc, _) => {
                            if widget.renaming {
                                widget.renaming = false;
                                widget.rename_text.clear();
                            } else if widget.filtering {
                                widget.filtering = false;
                                widget.filter_text.clear();
                                widget.rebuild_from_filter();
                            } else if widget.confirm_delete_path.is_some() {
                                widget.confirm_delete_path = None;
                            } else if widget.confirm_resume_path.is_some() {
                                widget.confirm_resume_path = None;
                            } else {
                                selected = None;
                                done = true;
                            }
                            request_frame.schedule_frame();
                        }
                        _ => {}
                    }
                }
                TuiEvent::Draw => {
                    let _ = tui.draw(u16::MAX, |frame| {
                        let area = frame.area();
                        Clear.render(area, frame.buffer_mut());
                        frame.render_widget(&mut widget, area);
                    });
                }
                _ => {}
            }
        } else {
            break;
        }
    }
    Ok(selected)
}
