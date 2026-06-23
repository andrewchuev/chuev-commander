//! Command-line widget — the single input line shown between the file panels
//! and the F-key status bar.
//!
//! Layout: `<dim-path>> <white-input><dim-ghost-hint>`
//!
//! The real terminal cursor is placed at the end of the typed input via
//! `frame.set_cursor_position`. A dim "ghost" span shows the history hint
//! (most-recent matching entry) so the user can accept it with →.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::CmdLine;
use crate::theme::Theme;
use crate::vfs::VfsPath;

/// `show_cursor` should be `true` only when no popup is covering the cmdline.
pub fn render_cmdline(
    frame:       &mut Frame,
    cmdline:     &CmdLine,
    panel_path:  &VfsPath,
    area:        Rect,
    theme:       &Theme,
    show_cursor: bool,
) {
    let path = panel_path.display_string();
    let prompt_full = format!(" {path}> ");

    let avail      = area.width as usize;
    let prompt_max = avail.saturating_sub(20);

    // Truncate a very long path from the left with "…"
    let prompt = if prompt_full.chars().count() > prompt_max && prompt_max > 2 {
        let skip = prompt_full.chars().count() - prompt_max + 1;
        let byte = prompt_full
            .char_indices()
            .nth(skip)
            .map(|(i, _)| i)
            .unwrap_or(prompt_full.len());
        format!("…{}", &prompt_full[byte..])
    } else {
        prompt_full
    };

    let prompt_w    = prompt.chars().count();
    // Reserve 1 col for cursor itself (it occupies a real terminal cell)
    let input_avail = avail.saturating_sub(prompt_w + 1);

    let input       = &cmdline.input;
    let input_chars = input.chars().count();

    // Show only the tail of the input when it overflows so cursor stays visible
    let display_input = if input_chars > input_avail {
        let skip = input_chars - input_avail;
        let byte = input.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
        &input[byte..]
    } else {
        input.as_str()
    };
    let display_input_chars = display_input.chars().count();

    // Ghost hint — the suffix from the most-recent matching history entry
    let hint_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM);

    let hint_chars_avail = input_avail.saturating_sub(display_input_chars);
    let hint_span = match cmdline.history_hint() {
        Some(hint) if hint_chars_avail > 0 => {
            let hint_display: String = hint.chars().take(hint_chars_avail).collect();
            Span::styled(hint_display, hint_style)
        }
        _ => Span::raw(""),
    };

    let spans = vec![
        Span::styled(prompt, theme.cmdline_path),
        Span::styled(display_input.to_owned(), theme.cmdline_text),
        hint_span,
    ];

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // Place the real terminal cursor at the end of the typed input
    if show_cursor {
        let cursor_x = area.x + prompt_w as u16 + display_input_chars as u16;
        let cursor_y = area.y;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}
