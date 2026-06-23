//! Menu bar and dropdown rendering.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::Popup;
use crate::menu::{menu_entries, MenuItem, MENU_TITLES};
use crate::theme::Theme;

/// Render the menu bar and (if open) the dropdown overlay.
pub fn render_menu(frame: &mut Frame, popup: &Popup, area: Rect, theme: &Theme) {
    let Popup::Menu { top_idx, sub_idx, open } = popup else { return };
    let (top_idx, sub_idx, open) = (*top_idx, *sub_idx, *open);

    render_menu_bar(frame, top_idx, area, theme);

    if open {
        let (x_offset, _) = title_x(top_idx, area);
        render_dropdown(frame, top_idx, sub_idx, x_offset, area, theme);
    }
}

// ── Menu bar (top row) ─────────────────────────────────────────────────────

fn render_menu_bar(frame: &mut Frame, top_idx: usize, area: Rect, theme: &Theme) {
    let bar_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };

    let mut spans: Vec<Span> = Vec::new();

    for (i, &title) in MENU_TITLES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", theme.menu_bar));
        }
        let style = if i == top_idx { theme.menu_bar_active } else { theme.menu_bar };
        spans.push(Span::styled(format!(" {title} "), style));
    }

    // Fill the rest of the bar
    let used: usize = MENU_TITLES
        .iter()
        .enumerate()
        .map(|(i, t)| t.len() + 2 + if i > 0 { 1 } else { 0 })
        .sum();
    let fill_w = (area.width as usize).saturating_sub(used);
    spans.push(Span::styled(" ".repeat(fill_w), theme.menu_bar));

    frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
}

// ── Dropdown ───────────────────────────────────────────────────────────────

/// Returns the x-offset of the given top-level title within the menu bar.
fn title_x(top_idx: usize, area: Rect) -> (u16, u16) {
    let mut x = area.x;
    for (i, &title) in MENU_TITLES.iter().enumerate() {
        let w = title.len() as u16 + 2; // " Title "
        if i == top_idx {
            return (x, w);
        }
        x += w + 1; // +1 for "│"
    }
    (area.x, 8)
}

fn render_dropdown(
    frame:    &mut Frame,
    top_idx:  usize,
    sub_idx:  usize,
    x_offset: u16,
    area:     Rect,
    theme:    &Theme,
) {
    let items = menu_entries(top_idx);
    if items.is_empty() { return; }

    // Compute inner width: widest label + gap + shortcut
    let inner_w = items
        .iter()
        .map(|e| match e {
            MenuItem::Entry { label, shortcut, .. } => {
                label.len() + 2 + if shortcut.is_empty() { 0 } else { shortcut.len() + 2 }
            }
            MenuItem::Separator => 1,
        })
        .max()
        .unwrap_or(10)
        .max(10);

    let drop_w = inner_w as u16 + 2; // +2 for borders
    let drop_h = items.len() as u16 + 2; // +2 for borders

    // Don't run off the right edge
    let x = x_offset.min(area.width.saturating_sub(drop_w));
    let y = area.y + 1; // just below the menu bar

    let drop_area = Rect { x, y, width: drop_w, height: drop_h };
    frame.render_widget(Clear, drop_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.menu_border)
        .style(theme.menu_item);

    let inner = block.inner(drop_area);
    frame.render_widget(block, drop_area);

    let iw = inner.width as usize;

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, entry)| match entry {
            MenuItem::Separator => {
                let sep = "─".repeat(iw);
                ListItem::new(Span::styled(sep, theme.menu_border))
            }
            MenuItem::Entry { label, shortcut, .. } => {
                let is_sel = i == sub_idx;
                let row_style = if is_sel { theme.menu_item_selected } else { theme.menu_item };
                let sc_style  = if is_sel { theme.menu_item_selected } else { theme.menu_shortcut };

                if shortcut.is_empty() {
                    let text = format!(" {:<width$}", label, width = iw.saturating_sub(1));
                    ListItem::new(Span::styled(text, row_style))
                } else {
                    let sc_len    = shortcut.len() + 1; // "Shortcut "
                    let label_w   = iw.saturating_sub(sc_len + 1);
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {:<width$}", label, width = label_w), row_style),
                        Span::styled(format!("{} ", shortcut), sc_style),
                    ]))
                }
            }
        })
        .collect();

    frame.render_widget(List::new(list_items), inner);
}
