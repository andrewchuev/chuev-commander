//! Popup (modal) rendering.
//!
//! Every popup is rendered over a `Clear` widget so it paints a clean
//! rectangle on top of whatever is behind it — no bleed-through from panels.
//!
//! To add a new popup variant: add it to `app::Popup` and add a render arm
//! to `render_top_popup` below.

use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

/// Maximum rows shown in the history suggestion popup.
const HISTORY_POPUP_MAX_ITEMS: u16 = 15;

use crate::app::{Popup, ViewerMode};
use crate::theme::Theme;
use crate::ui::menu::render_menu;

/// Render the topmost popup from the stack (called by `ui::render` when the
/// stack is non-empty).
///
/// `cmdline_area` is used to position the `HistoryMenu` popup just above the
/// command line.
pub fn render_top_popup(frame: &mut Frame, popup: &Popup, full_area: Rect, theme: &Theme) {
    match popup {
        Popup::Error(msg) => render_error(frame, msg, full_area, theme),
        Popup::Confirm { title, message, .. } => {
            render_confirm(frame, title, message, full_area, theme)
        }
        Popup::Input { title, prompt, value, .. } => {
            render_input(frame, title, prompt, value, full_area, theme)
        }
        Popup::Progress { title, source_name, bytes_done, bytes_total, .. } => {
            render_progress(frame, title, source_name, *bytes_done, *bytes_total, full_area, theme)
        }
        Popup::Viewer { title, content, mode, scroll_y, text_line_count } => {
            render_viewer(frame, title, content, *text_line_count, mode, *scroll_y, full_area, theme)
        }
        Popup::Menu { .. } => render_menu(frame, popup, full_area, theme),
        Popup::BookmarkManager { entries, selected } => {
            render_bookmark_manager(frame, entries, *selected, full_area, theme)
        }
    }
}

fn render_error(frame: &mut Frame, message: &str, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(50, 25, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Error ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(theme.popup_error_border);

    let text = Paragraph::new(format!("{message}\n\n[Esc] Close"))
        .block(block)
        .style(theme.popup_text)
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center);

    frame.render_widget(text, popup_area);
}

fn render_confirm(frame: &mut Frame, title: &str, message: &str, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(50, 30, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(theme.popup_border);

    let body = format!("{message}\n\n[Enter] Confirm   [Esc] Cancel");
    let text = Paragraph::new(body)
        .block(block)
        .style(theme.popup_text)
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center);

    frame.render_widget(text, popup_area);
}

fn render_input(frame: &mut Frame, title: &str, prompt: &str, value: &str, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(54, 22, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(theme.popup_border);

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Lay out rows: top padding, prompt, input field, gap, hint.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top padding
            Constraint::Length(1), // prompt
            Constraint::Length(1), // input field
            Constraint::Length(1), // gap
            Constraint::Length(1), // hint
            Constraint::Min(0),    // remaining space
        ])
        .split(inner);

    // ── Prompt line ───────────────────────────────────────────────────────
    frame.render_widget(
        Paragraph::new(format!(" {prompt}")).style(theme.popup_text),
        rows[1],
    );

    // ── Input field with blinking-cursor simulation ───────────────────────
    let field_style  = theme.popup_input_field;
    let cursor_style = theme.popup_input_cursor;

    // field_width is in display columns (width - 2 for the leading space + cursor)
    let field_width  = inner.width.saturating_sub(2) as usize;
    let char_count   = value.chars().count();
    let display_val  = if char_count >= field_width {
        // Show the last (field_width - 1) characters so the cursor column is visible
        let skip      = char_count - field_width.saturating_sub(1);
        let byte_off  = value.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
        &value[byte_off..]
    } else {
        value
    };
    // Remaining columns after leading space + text + cursor
    let visible_chars = display_val.chars().count();
    let padding = " ".repeat(field_width.saturating_sub(visible_chars + 1));

    let line = Line::from(vec![
        Span::styled(" ", field_style),
        Span::styled(display_val.to_owned(), field_style),
        Span::styled("█", cursor_style),
        Span::styled(padding, field_style),
    ]);
    frame.render_widget(Paragraph::new(line), rows[2]);

    // ── Hint ─────────────────────────────────────────────────────────────
    frame.render_widget(
        Paragraph::new(" [Enter] OK   [Esc] Cancel").style(theme.popup_hint),
        rows[4],
    );
}

fn render_progress(
    frame:       &mut Frame,
    title:       &str,
    source_name: &str,
    bytes_done:  u64,
    bytes_total: u64,
    area:        Rect,
    theme:       &Theme,
) {
    let popup_area = centered_rect(60, 28, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(theme.popup_border);

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // File name
    let name_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 };
    let max_name  = inner.width.saturating_sub(2) as usize;
    let display   = if source_name.len() > max_name {
        format!("…{}", &source_name[source_name.len() - max_name.saturating_sub(1)..])
    } else {
        source_name.to_owned()
    };
    frame.render_widget(
        Paragraph::new(format!(" {display}")).style(theme.popup_text),
        name_area,
    );

    // Gauge
    let pct = if bytes_total > 0 {
        ((bytes_done as f64 / bytes_total as f64) * 100.0) as u16
    } else {
        0
    };
    let label = format!(
        "{}%  ({} / {})",
        pct,
        crate::ui::panels::format_size(bytes_done),
        crate::ui::panels::format_size(bytes_total)
    );
    let gauge_area = Rect { x: inner.x, y: inner.y + 3, width: inner.width, height: 1 };
    frame.render_widget(
        Gauge::default()
            .gauge_style(theme.popup_gauge)
            .percent(pct)
            .label(label),
        gauge_area,
    );

    // Hint
    let hint_area = Rect { x: inner.x, y: inner.y + 5, width: inner.width, height: 1 };
    frame.render_widget(
        Paragraph::new(" [Esc] Cancel").style(theme.popup_hint),
        hint_area,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_viewer(
    frame:           &mut Frame,
    title:           &str,
    content:         &[u8],
    text_line_count: usize,
    mode:            &ViewerMode,
    scroll_y:        usize,
    area:            Rect,
    theme:           &Theme,
) {
    let popup_area = centered_rect(96, 94, area);
    frame.render_widget(Clear, popup_area);

    let mode_label = match mode { ViewerMode::Text => "Text", ViewerMode::Hex => "Hex" };
    let block = Block::default()
        .title(format!(" {} ─ [{mode_label}] ", title))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_style(theme.popup_border);

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Reserve the last row for the hint/status line
    let content_h    = inner.height.saturating_sub(1);
    let content_area = Rect { height: content_h, ..inner };
    let status_area  = Rect { y: inner.y + content_h, height: 1, ..inner };

    match mode {
        ViewerMode::Text => render_viewer_text(frame, content, scroll_y, content_area, theme),
        ViewerMode::Hex  => render_viewer_hex(frame, content, scroll_y, content_area),
    }

    // Use the pre-computed line count (passed in from Popup::Viewer) to avoid
    // re-scanning the entire content on every render frame.
    let total = match mode {
        ViewerMode::Text => text_line_count.max(1),
        ViewerMode::Hex  => content.len().div_ceil(16).max(1),
    };
    let hint = format!(
        " [F3/Esc/Q] Close   [H] Toggle Hex/Text   {:>5}/{total} ",
        scroll_y + 1,
    );
    frame.render_widget(
        Paragraph::new(hint).style(theme.popup_hint),
        status_area,
    );
}

fn render_viewer_text(frame: &mut Frame, content: &[u8], scroll_y: usize, area: Rect, theme: &Theme) {
    let text = String::from_utf8_lossy(content);
    let w    = area.width as usize;

    let lines: Vec<Line> = text
        .lines()
        .skip(scroll_y)
        .take(area.height as usize)
        .map(|line| {
            // Truncate to visible width (char-boundary safe)
            let end = line
                .char_indices()
                .nth(w)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            Line::from(Span::raw(line[..end].to_owned()))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(theme.popup_text),
        area,
    );
}

fn render_viewer_hex(frame: &mut Frame, content: &[u8], scroll_y: usize, area: Rect) {
    const COLS: usize = 16;

    let lines: Vec<Line> = content
        .chunks(COLS)
        .enumerate()
        .skip(scroll_y)
        .take(area.height as usize)
        .map(|(row, chunk)| {
            let offset = row * COLS;

            // Build hex part
            let mut hex = String::with_capacity(50);
            for (i, b) in chunk.iter().enumerate() {
                if i == 8 { hex.push(' '); }   // middle gap
                hex.push_str(&format!("{b:02x} "));
            }
            // Pad to a fixed 49-char field
            let used = chunk.len() * 3 + if chunk.len() > 8 { 1 } else { 0 };
            hex.push_str(&" ".repeat(49 - used));

            let ascii: String = chunk.iter()
                .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                .collect();

            Line::from(vec![
                Span::styled(
                    format!("{offset:08x}  "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(hex, Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!(" |{ascii}|"),
                    Style::default().fg(Color::Yellow),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

// ─────────────────────────────────────────────────────────────────────────────
// History suggestion popup (vertical, not on popup_stack)
// ─────────────────────────────────────────────────────────────────────────────

/// Vertical list of history suggestions overlaid above the command line.
/// Called directly from `ui::render` (not through `render_top_popup`).
pub fn render_history_popup(
    frame:        &mut Frame,
    matches:      &[String],
    selected_idx: usize,
    cmdline_area: Rect,
    full_area:    Rect,
    theme:        &Theme,
) {
    if matches.is_empty() { return; }

    // Item 0 is the blank "execute-typed" sentinel; items 1..=n map to matches[0..n-1]
    let total_items = matches.len() + 1;
    let n_items   = (total_items as u16).min(HISTORY_POPUP_MAX_ITEMS);
    let hint_rows = 1_u16;
    let sep_rows  = 1_u16;
    let needed    = n_items + hint_rows + sep_rows;

    // Available vertical space above cmdline
    let avail = cmdline_area.y.saturating_sub(full_area.y);
    if avail == 0 { return; }

    let popup_h = needed.min(avail);
    let list_h  = popup_h.saturating_sub(hint_rows + sep_rows);

    let popup_y = cmdline_area.y.saturating_sub(popup_h);
    let popup_area = Rect {
        x: full_area.x, y: popup_y,
        width: full_area.width, height: popup_h,
    };

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Block::default().style(theme.history_popup_bg),
        popup_area,
    );

    let list_area = Rect { height: list_h, ..popup_area };
    let sep_area  = Rect { y: popup_area.y + list_h, height: sep_rows, ..popup_area };
    let hint_area = Rect { y: popup_area.y + list_h + sep_rows, height: hint_rows, ..popup_area };

    // Scroll so the selected item stays in view
    let scroll = if selected_idx >= list_h as usize {
        selected_idx + 1 - list_h as usize
    } else {
        0
    };

    let item_w = popup_area.width.saturating_sub(4) as usize; // room for " ► "

    let lines: Vec<Line> = (0..total_items)
        .skip(scroll)
        .take(list_h as usize)
        .map(|i| {
            let is_sel = i == selected_idx;
            let style  = if is_sel { theme.history_popup_selected } else { theme.history_popup_item };
            let marker = if is_sel { "►" } else { " " };
            // idx 0 = blank sentinel; idx > 0 = matches[i-1]
            let display = if i == 0 {
                String::new()
            } else {
                let cmd = &matches[i - 1];
                if cmd.chars().count() > item_w {
                    let t: String = cmd.chars().take(item_w.saturating_sub(1)).collect();
                    format!("{t}…")
                } else {
                    cmd.clone()
                }
            };
            Line::from(Span::styled(format!(" {marker} {display}"), style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), list_area);

    // Separator — Block with top border avoids a per-frame String allocation
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(theme.history_popup_sep),
        sep_area,
    );

    // Key hint
    frame.render_widget(
        Paragraph::new(format!(
            " ↑↓ navigate   Enter execute   Esc close   Shift+Del delete   ({}/{})",
            selected_idx + 1, total_items
        ))
        .style(theme.history_popup_hint),
        hint_area,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Bookmark-manager popup
// ─────────────────────────────────────────────────────────────────────────────

fn render_bookmark_manager(
    frame:    &mut Frame,
    entries:  &[(u8, PathBuf)],
    selected: usize,
    area:     Rect,
    theme:    &Theme,
) {
    let popup_area = centered_rect(72, 65, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Folder Bookmarks ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(theme.popup_border);

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if entries.is_empty() {
        let msg = Paragraph::new(
            "\n  No bookmarks yet.\n\
               \n  Press [Ins] to add the current folder as a bookmark.\n\
               \n  [Esc] Close"
        )
        .style(theme.popup_text);
        frame.render_widget(msg, inner);
        return;
    }

    let hint_h    = 1_u16;
    let list_h    = inner.height.saturating_sub(hint_h + 1);
    let list_area = Rect { height: list_h, ..inner };
    let hint_area = Rect { y: inner.y + list_h + 1, height: hint_h, ..inner };

    let scroll = if selected >= list_h as usize { selected + 1 - list_h as usize } else { 0 };
    let path_w = inner.width.saturating_sub(12) as usize; // leave room for "► Ctrl+N  "

    let lines: Vec<Line> = entries.iter()
        .enumerate()
        .skip(scroll)
        .take(list_h as usize)
        .map(|(i, (n, path))| {
            let is_sel = i == selected;
            let style  = if is_sel {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                theme.popup_text
            };
            let marker = if is_sel { "►" } else { " " };
            let raw    = path.display().to_string();
            let display = if raw.chars().count() > path_w {
                let skip = raw.chars().count() - path_w + 1;
                let byte = raw.char_indices().nth(skip).map(|(b, _)| b).unwrap_or(raw.len());
                format!("…{}", &raw[byte..])
            } else {
                raw
            };
            Line::from(Span::styled(
                format!("{marker} Ctrl+{n}  {display}"),
                style,
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), list_area);
    frame.render_widget(
        Paragraph::new(" [Enter] Go   [Ins] Add   [Del] Remove   [Esc] Close")
            .style(theme.popup_hint),
        hint_area,
    );
}

/// Returns a `Rect` centred within `area` at the given percentage dimensions.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let margin_y = (100 - percent_y) / 2;
    let margin_x = (100 - percent_x) / 2;

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(margin_y),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(margin_y),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin_x),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(margin_x),
        ])
        .split(vert[1])[1]
}
