//! Output buffer view — rendered in the panels area when Ctrl+O hides panels.
//!
//! Displays accumulated shell command output with no border or title.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::theme::Theme;

pub fn render_output(frame: &mut Frame, lines: &[String], scroll: &mut usize, area: Rect, theme: &Theme) {
    let visible_height = area.height as usize;

    // Clamp scroll so we never go past the last possible first-visible line
    let max_scroll = lines.len().saturating_sub(visible_height);
    if *scroll > max_scroll {
        *scroll = max_scroll;
    }

    let visible: Vec<Line> = lines
        .iter()
        .skip(*scroll)
        .take(visible_height)
        .map(|l| render_line(l, theme))
        .collect();

    frame.render_widget(Paragraph::new(visible), area);

    // Scroll position hint in the bottom-right corner
    if lines.len() > visible_height {
        let pct = ((*scroll + visible_height).min(lines.len()) * 100) / lines.len();
        let hint = format!(" {}/{}  {}% ", *scroll + 1, lines.len(), pct);
        let hint_len = hint.len() as u16;
        if area.width >= hint_len {
            let hint_area = Rect {
                x:      area.x + area.width - hint_len,
                y:      area.y + area.height - 1,
                width:  hint_len,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, theme.output_border)),
                hint_area,
            );
        }
    }
}

fn render_line(line: &str, theme: &Theme) -> Line<'static> {
    if line.starts_with('[') && line.contains("]$ ") {
        let owned = line.to_owned();
        if let Some(sep) = owned.find("]$ ") {
            let path_part = format!("{}]", &owned[..sep]);
            let cmd_part  = format!("$ {}", &owned[sep + 3..]);
            return Line::from(vec![
                Span::styled(path_part, theme.output_cmd_path),
                Span::styled(cmd_part,  theme.output_cmd_text),
            ]);
        }
    }
    Line::from(Span::raw(line.to_owned()))
}
