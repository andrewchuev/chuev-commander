//! Bottom status / function-key bar (one line, spans full width).
//!
//! Each of the 10 F-key slots gets an equal share of the terminal width;
//! the last slot absorbs any remainder from integer division.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::theme::Theme;

/// Labels paired with each F-key, in order F1–F10.
const FKEYS: &[(&str, &str)] = &[
    ("1",  "Help"),
    ("2",  "Menu"),
    ("3",  "View"),
    ("4",  "Edit"),
    ("5",  "Copy"),
    ("6",  "RenMov"),
    ("7",  "MkDir"),
    ("8",  "Delete"),
    ("9",  "PullDn"),
    ("10", "Quit"),
];

pub fn render_status_bar(frame: &mut Frame, area: Rect, theme: &Theme) {
    let total_w = area.width as usize;
    let n       = FKEYS.len();

    // Each slot is separated by a 1-char black gap; 9 separators for 10 items.
    let sep_count   = n - 1;
    let content_w   = total_w.saturating_sub(sep_count);
    let slot_w      = content_w / n;
    let remainder   = content_w.saturating_sub(slot_w * n);

    let sep_span = Span::styled(" ", theme.status_sep);

    let mut spans: Vec<Span> = Vec::with_capacity(n * 3);
    for (i, (num, label)) in FKEYS.iter().enumerate() {
        let key_str = format!("F{num}");
        let key_len = key_str.len();
        let this_w  = slot_w + if i == n - 1 { remainder } else { 0 };
        let label_w = this_w.saturating_sub(key_len);

        spans.push(Span::styled(key_str, theme.status_key));
        spans.push(Span::styled(format!("{:<width$}", label, width = label_w), theme.status_label));
        if i < n - 1 {
            spans.push(sep_span.clone());
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
