//! Popup (modal) rendering.
//!
//! Every popup is rendered over a `Clear` widget so it paints a clean
//! rectangle on top of whatever is behind it — no bleed-through from panels.
//!
//! To add a new popup variant: add it to `app::Popup` and add a render arm
//! to `render_top_popup` below.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

use crate::app::{Popup, ViewerMode};
use crate::theme::Theme;
use crate::ui::menu::render_menu;

/// Render the topmost popup from the stack (called by `ui::render` when the
/// stack is non-empty).
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
        Popup::Viewer { title, content, mode, scroll_y } => {
            render_viewer(frame, title, content, mode, *scroll_y, full_area, theme)
        }
        Popup::Menu { .. } => render_menu(frame, popup, full_area, theme),
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

    // ── Prompt line ───────────────────────────────────────────────────────
    let prompt_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 };
    frame.render_widget(
        Paragraph::new(format!(" {prompt}")).style(theme.popup_text),
        prompt_area,
    );

    // ── Input field with blinking-cursor simulation ───────────────────────
    let field_area  = Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: 1 };
    let field_style  = theme.popup_input_field;
    let cursor_style = theme.popup_input_cursor;

    let field_width = inner.width.saturating_sub(2) as usize;
    let display_val = if value.len() >= field_width {
        &value[value.len() - field_width.saturating_sub(1)..]
    } else {
        value
    };
    let padding = " ".repeat(field_width.saturating_sub(display_val.len() + 1));

    let line = Line::from(vec![
        Span::styled(" ", field_style),
        Span::styled(display_val.to_owned(), field_style),
        Span::styled("█", cursor_style),
        Span::styled(padding, field_style),
    ]);
    frame.render_widget(Paragraph::new(line), field_area);

    // ── Hint ─────────────────────────────────────────────────────────────
    let hint_area = Rect { x: inner.x, y: inner.y + 4, width: inner.width, height: 1 };
    frame.render_widget(
        Paragraph::new(" [Enter] OK   [Esc] Cancel").style(theme.popup_hint),
        hint_area,
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

fn render_viewer(
    frame:    &mut Frame,
    title:    &str,
    content:  &[u8],
    mode:     &ViewerMode,
    scroll_y: usize,
    area:     Rect,
    theme:    &Theme,
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
    let content_h  = inner.height.saturating_sub(1);
    let content_area = Rect { height: content_h, ..inner };
    let status_area  = Rect { y: inner.y + content_h, height: 1, ..inner };

    match mode {
        ViewerMode::Text => render_viewer_text(frame, content, scroll_y, content_area, theme),
        ViewerMode::Hex  => render_viewer_hex(frame, content, scroll_y, content_area),
    }

    let total = match mode {
        ViewerMode::Text => String::from_utf8_lossy(content).lines().count().max(1),
        ViewerMode::Hex  => ((content.len() + 15) / 16).max(1),
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
