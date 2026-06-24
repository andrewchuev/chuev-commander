//! Command-line widget — the single input line shown between the file panels
//! and the F-key status bar.
//!
//! Layout: `<dim-path>> <white-input>`
//!
//! The real terminal cursor tracks `CmdLine::cursor_pos`.  When a selection is
//! active the selected region is highlighted with a blue background.
//! History suggestions are shown in the history popup overlay above this line.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
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
    let cursor_char = input[..cmdline.cursor_pos].chars().count();

    // Scroll the view so the cursor is always within [0, input_avail).
    let scroll_chars: usize = if cursor_char < input_avail {
        0
    } else {
        cursor_char + 1 - input_avail
    };

    // Byte range of the visible portion of the input
    let vis_start_byte = nth_char_byte(input, scroll_chars);
    let vis_end_char   = (scroll_chars + input_avail).min(input_chars);
    let vis_end_byte   = nth_char_byte(input, vis_end_char);
    let display_input  = &input[vis_start_byte..vis_end_byte];

    let cursor_col = cursor_char - scroll_chars;

    // ── Selection bounds (absolute char indices) ──────────────────────────
    let sel_range: Option<(usize, usize)> = cmdline.selection_anchor.and_then(|anchor| {
        let anchor_char = input[..anchor].chars().count();
        let (s, e) = if anchor_char <= cursor_char {
            (anchor_char, cursor_char)
        } else {
            (cursor_char, anchor_char)
        };
        if s == e { None } else { Some((s, e)) }
    });

    // ── Input spans (plain or with selection highlight) ───────────────────
    let sel_style = Style::default().bg(Color::Blue).fg(Color::White);

    let input_spans: Vec<Span<'static>> = match sel_range {
        Some((sel_s, sel_e)) => {
            // Clamp selection to the visible window
            let vis_s = sel_s.max(scroll_chars);
            let vis_e = sel_e.min(vis_end_char);
            if vis_s < vis_e {
                let local_s   = vis_s - scroll_chars;
                let local_e   = vis_e - scroll_chars;
                let pre_end_b = nth_char_byte(display_input, local_s);
                let sel_end_b = nth_char_byte(display_input, local_e);

                let pre  = &display_input[..pre_end_b];
                let sel  = &display_input[pre_end_b..sel_end_b];
                let post = &display_input[sel_end_b..];

                let mut s: Vec<Span<'static>> = Vec::with_capacity(3);
                if !pre.is_empty()  { s.push(Span::styled(pre.to_owned(),  theme.cmdline_text)); }
                if !sel.is_empty()  { s.push(Span::styled(sel.to_owned(),  sel_style)); }
                if !post.is_empty() { s.push(Span::styled(post.to_owned(), theme.cmdline_text)); }
                if s.is_empty()     { s.push(Span::raw("")); }
                s
            } else {
                vec![Span::styled(display_input.to_owned(), theme.cmdline_text)]
            }
        }
        None => vec![Span::styled(display_input.to_owned(), theme.cmdline_text)],
    };

    // History suggestions are shown in the popup overlay; no inline ghost hint.
    let hint_span: Span<'static> = Span::raw("");

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(input_spans.len() + 2);
    spans.push(Span::styled(prompt, theme.cmdline_path));
    spans.extend(input_spans);
    spans.push(hint_span);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // Place the real terminal cursor at the current cursor position
    if show_cursor {
        let cursor_x = area.x + prompt_w as u16 + cursor_col as u16;
        let cursor_y = area.y;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Returns the byte offset of the `n`-th character boundary in `s`.
/// Returns `s.len()` when `n` >= the number of characters.
fn nth_char_byte(s: &str, n: usize) -> usize {
    s.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}
