use std::path::{Path, PathBuf};

// Interactive picker for resuming past sessions.
//
// Keys:
// - Up/Down: move selection
// - Enter: resume the selected session
// - Esc/Ctrl-C: cancel without resuming
//
// Rows show: local timestamp → 8-char id prefix → title (session name or
// first line of instructions). The file path is shown dimmed under each row.

use chrono::{DateTime, Utc};
use codex_core::rollout::read_session_header_and_state;
use color_eyre::eyre::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::Widget;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, WidgetRef};
use tokio_stream::StreamExt;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::bottom_pane::{GenericDisplayRow, MAX_POPUP_ROWS, ScrollState, render_rows};
use crate::tui::{FrameRequester, Tui, TuiEvent};

#[derive(Debug, Clone)]
struct SessionEntry {
    id: Uuid,
    when: DateTime<Utc>,
    title: String,
    path: PathBuf,
}

fn read_header(path: &Path) -> std::io::Result<Option<SessionEntry>> {
    let (meta, state) = match read_session_header_and_state(path) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
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
    Ok(Some(SessionEntry {
        id: meta.id,
        when,
        title,
        path: path.to_path_buf(),
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
        if let Ok(Some(header)) = read_header(&entry.path()) {
            out.push(header);
        }
    }
    // newest first
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out.truncate(limit);
    Ok(out)
}

struct SessionsPickerWidget {
    rows: Vec<GenericDisplayRow>,
    paths: Vec<PathBuf>,
    state: ScrollState,
}

impl SessionsPickerWidget {
    fn from_entries(entries: &[SessionEntry]) -> Self {
        let mut rows = Vec::new();
        let mut paths = Vec::new();
        for e in entries {
            let ts = e.when.format("%Y-%m-%d %H:%M:%S");
            let name = format!(
                "{ts}  {}  {}",
                &e.id.as_hyphenated().to_string()[..8],
                e.title
            );
            rows.push(GenericDisplayRow {
                name,
                match_indices: None,
                is_current: false,
                description: Some(e.path.to_string_lossy().to_string()),
            });
            paths.push(e.path.clone());
        }
        Self {
            rows,
            paths,
            state: ScrollState::new(),
        }
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.state
            .selected_idx
            .and_then(|i| self.paths.get(i))
            .cloned()
    }
}

impl WidgetRef for &SessionsPickerWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
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

        let title = Paragraph::new(Line::from(vec![
            "Resume a Previous Session".bold().cyan(),
            " — newest first (limit)".into(),
        ]))
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
            MAX_POPUP_ROWS.max(chunks[1].height as usize),
            false,
        );

        let hint = Paragraph::new(Line::from(vec![
            "↑/↓".into(),
            " move  ".dim(),
            "Enter".into(),
            " resume  ".dim(),
            "Esc".into(),
            " cancel".dim(),
        ]))
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::NONE));
        hint.render(chunks[2], buf);
    }
}

pub(crate) async fn run_sessions_picker_app(
    tui: &mut Tui,
    codex_home: &Path,
    limit: usize,
) -> Result<Option<PathBuf>> {
    // Load entries from disk
    let entries = collect_recent_sessions(codex_home, limit).unwrap_or_default();

    // If nothing to show, return early
    if entries.is_empty() {
        return Ok(None);
    }

    let request_frame: FrameRequester = tui.frame_requester();
    let mut widget = SessionsPickerWidget::from_entries(&entries);
    widget.state.clamp_selection(widget.rows.len());
    widget
        .state
        .ensure_visible(widget.rows.len(), widget.rows.len().min(MAX_POPUP_ROWS));

    // Initial draw
    tui.draw(u16::MAX, |frame| {
        let area = frame.area();
        Clear.render(area, frame.buffer_mut());
        frame.render_widget_ref(&widget, area);
    })?;

    let mut selected: Option<PathBuf> = None;
    let mut done = false;
    let events = tui.event_stream();
    tokio::pin!(events);

    while !done {
        if let Some(ev) = events.next().await {
            match ev {
                TuiEvent::Key(key) => {
                    use crossterm::event::{KeyCode, KeyModifiers};
                    match (key.code, key.modifiers) {
                        (KeyCode::Up, _) => {
                            widget.state.move_up_wrap(widget.rows.len());
                            widget.state.ensure_visible(
                                widget.rows.len(),
                                widget.rows.len().min(MAX_POPUP_ROWS),
                            );
                            request_frame.schedule_frame();
                        }
                        (KeyCode::Down, _) => {
                            widget.state.move_down_wrap(widget.rows.len());
                            widget.state.ensure_visible(
                                widget.rows.len(),
                                widget.rows.len().min(MAX_POPUP_ROWS),
                            );
                            request_frame.schedule_frame();
                        }
                        (KeyCode::Enter, _) => {
                            selected = widget.selected_path();
                            done = true;
                        }
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            selected = None;
                            done = true;
                        }
                        _ => {}
                    }
                }
                TuiEvent::Draw => {
                    let _ = tui.draw(u16::MAX, |frame| {
                        let area = frame.area();
                        Clear.render(area, frame.buffer_mut());
                        frame.render_widget_ref(&widget, area);
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
