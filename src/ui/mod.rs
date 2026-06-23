//! Top-level render function.

pub mod cmdline;
pub mod menu;
pub mod output;
pub mod panels;
pub mod popups;
pub mod status;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, PanelSide};
use cmdline::render_cmdline;
use output::render_output;
use panels::render_panel;
use popups::render_top_popup;
use status::render_status_bar;

/// Draw the entire UI for a single frame.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area  = frame.area();
    let theme = &app.theme;

    // ── Vertical split ────────────────────────────────────────────────────
    //   panels/output  (fills remaining height)
    //   cmdline        (1 line)
    //   separator      (1 line — visual gap between prompt and F-keys)
    //   fkeys          (1 line)
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let panels_area  = vchunks[0];
    let cmdline_area = vchunks[1];
    let sep_area     = vchunks[2];
    let status_area  = vchunks[3];

    let panels_hidden = !app.left_panel_visible && !app.right_panel_visible;

    if panels_hidden {
        // ── Full output buffer view (Ctrl+O) ───────────────────────────────
        render_output(frame, &app.output_buffer, &mut app.output_scroll, panels_area, theme);
    } else {
        // ── File panels, optionally with an output strip below ────────────
        let (panel_rect, output_rect) = split_panels_output(panels_area, app.panels_height_percent);

        let (left_area, right_area) = panel_areas(panel_rect, app);

        // Store rects for mouse-click mapping
        app.layout.left_panel  = left_area;
        app.layout.right_panel = right_area;
        app.layout.output      = output_rect;

        if let Some(la) = left_area {
            render_panel(frame, &mut app.left_panel, la, app.active_panel == PanelSide::Left, theme);
        }
        if let Some(ra) = right_area {
            render_panel(frame, &mut app.right_panel, ra, app.active_panel == PanelSide::Right, theme);
        }

        if let Some(oa) = output_rect {
            render_output(frame, &app.output_buffer, &mut app.output_scroll, oa, theme);
        }
    }

    // ── Command line ──────────────────────────────────────────────────────
    let panel_path   = app.active_panel().current_path.clone();
    let show_cursor  = app.popup_stack.is_empty();
    render_cmdline(frame, &app.cmdline, &panel_path, cmdline_area, theme, show_cursor);

    // ── Separator between cmdline and F-keys ──────────────────────────────
    let sep_line = "─".repeat(sep_area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep_line, Style::default().fg(ratatui::style::Color::DarkGray))),
        sep_area,
    );

    // ── F-key status bar ──────────────────────────────────────────────────
    render_status_bar(frame, status_area, theme);

    // ── Popup stack — topmost entry paints over everything else ───────────
    if let Some(popup) = app.popup_stack.last() {
        let popup = popup.clone();
        render_top_popup(frame, &popup, area, theme);
    }
}

/// Split `area` vertically into a panels rect and an optional output rect.
///
/// When `height_pct` is 100 the entire area goes to panels and `output` is
/// `None`.  Otherwise `panels` gets `height_pct`% of the rows (minimum 3)
/// and the remainder goes to `output`.
fn split_panels_output(area: Rect, height_pct: u16) -> (Rect, Option<Rect>) {
    if height_pct >= 100 || area.height < 4 {
        return (area, None);
    }
    let total    = area.height;
    let panel_h  = ((total as u32 * height_pct as u32) / 100).max(3).min(total as u32 - 1) as u16;
    let output_h = total - panel_h;

    let panel_rect = Rect { height: panel_h, ..area };
    let output_rect = Rect { y: area.y + panel_h, height: output_h, ..area };
    (panel_rect, Some(output_rect))
}

fn panel_areas(area: Rect, app: &App) -> (Option<Rect>, Option<Rect>) {
    match (app.left_panel_visible, app.right_panel_visible) {
        (true, true) => {
            let w = app.left_panel_width_percent.clamp(10, 90);
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(w),
                    Constraint::Percentage(100 - w),
                ])
                .split(area);
            (Some(chunks[0]), Some(chunks[1]))
        }
        (true, false) => (Some(area), None),
        (false, true) => (None, Some(area)),
        (false, false) => (None, None),
    }
}
