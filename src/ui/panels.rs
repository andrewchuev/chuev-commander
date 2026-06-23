//! Panel widget — renders one directory listing inside a bordered block.
//!
//! ## Layout (inside the border)
//! ```
//! ┌─ /home/user  ↑Name  45.2 G free ──────────────────────────────┐
//! │ Name                          Size      Date       Perms       │
//! │──────────────────────────────────────────────────────────────  │
//! │▸ Documents/                   <DIR>  2024-01-15   drwxr-xr-x  │
//! │  README.md                   12.3K   2024-01-10   -rw-r--r--  │
//! │ …                                                              │
//! │ Search: readme_                                                 │
//! └────────────────────────────────────────────────────────────────┘
//! ```
//! All column widths are computed dynamically from the available width.

use chrono::{DateTime, Local};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{PanelState, SortColumn, SortOrder};
use crate::theme::Theme;

// ─────────────────────────────────────────────────────────────────────────────
// Column layout
// ─────────────────────────────────────────────────────────────────────────────

/// Computed column widths for a panel of a given character width.
struct ColumnLayout {
    /// Available width for the file name (after prefix and other columns).
    name_w:     usize,
    show_size:  bool,
    show_date:  bool,
    show_perms: bool,
}

impl ColumnLayout {
    const PREFIX: usize = 1; // 1-char type indicator: '/' dir  '*' exec  ' ' file
    const SIZE_W: usize = 8; // "  12.3K " right-aligned
    const DATE_W: usize = 11; // " 2024-01-15"
    const PERM_W: usize = 10; // " rwxr-xr-x"

    fn from_inner_width(w: usize) -> Self {
        let show_perms = w >= 70;
        let show_date  = w >= 52;
        let show_size  = w >= 36;

        let mut fixed = Self::PREFIX;
        if show_size  { fixed += Self::SIZE_W; }
        if show_date  { fixed += Self::DATE_W; }
        if show_perms { fixed += Self::PERM_W; }

        let name_w = w.saturating_sub(fixed);

        Self { name_w, show_size, show_date, show_perms }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public render function
// ─────────────────────────────────────────────────────────────────────────────

pub fn render_panel(frame: &mut Frame, panel: &mut PanelState, area: Rect, focused: bool, theme: &Theme) {
    let border_style = if focused {
        theme.panel_border_active
    } else {
        theme.panel_border_inactive
    };

    let title = build_title(panel, focused);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let total_h = inner.height as usize;
    if total_h < 3 {
        return; // too narrow to draw anything useful
    }

    let layout = ColumnLayout::from_inner_width(inner.width as usize);
    let search_active  = panel.search_mode;

    // Row allocation:
    //   row 0:        column header
    //   row 1:        separator line
    //   rows 2..h-1:  file list  (or 2..h-2 if search bar is visible)
    //   last row:     quick-search bar (when active)
    let header_rows = 2usize; // header + separator
    let search_rows = if search_active { 1usize } else { 0 };
    let list_height = total_h.saturating_sub(header_rows + search_rows);

    // ── Scroll adjustment ─────────────────────────────────────────────────
    panel.ensure_visible(list_height);

    // ── Column header ────────────────────────────────────────────────────
    let header_area = Rect {
        x:      inner.x,
        y:      inner.y,
        width:  inner.width,
        height: 1,
    };
    render_column_header(frame, &layout, header_area, panel.sort_column, panel.sort_order, theme);

    // ── Separator ─────────────────────────────────────────────────────────
    let sep_area  = Rect { y: inner.y + 1, height: 1, ..header_area };
    let sep_line  = "─".repeat(inner.width as usize);
    let sep_style = if focused { theme.panel_sep_active } else { theme.panel_sep_inactive };
    frame.render_widget(
        Paragraph::new(sep_line).style(sep_style),
        sep_area,
    );

    // ── File list ─────────────────────────────────────────────────────────
    let list_area = Rect {
        x:      inner.x,
        y:      inner.y + header_rows as u16,
        width:  inner.width,
        height: list_height as u16,
    };

    let items: Vec<ListItem> = panel
        .entries
        .iter()
        .enumerate()
        .skip(panel.scroll_offset)
        .take(list_height)
        .map(|(abs_idx, entry)| {
            let is_cursor = abs_idx == panel.selected_index;
            let is_marked = panel.selected_names.contains(&entry.name);
            build_list_item(
                entry,
                &layout,
                is_cursor,
                is_marked,
                focused,
                &panel.quick_search,
                theme,
            )
        })
        .collect();

    frame.render_widget(List::new(items), list_area);

    // ── Quick-search bar ──────────────────────────────────────────────────
    if search_active {
        let search_area = Rect {
            x:      inner.x,
            y:      inner.y + total_h as u16 - 1,
            width:  inner.width,
            height: 1,
        };
        render_search_bar(frame, &panel.quick_search, search_area);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Block title
// ─────────────────────────────────────────────────────────────────────────────

fn build_title(panel: &PanelState, focused: bool) -> String {
    let path = panel.current_path.display_string();

    let sort_indicator = match (panel.sort_column, panel.sort_order) {
        (SortColumn::Name,     SortOrder::Asc)  => "↑Name",
        (SortColumn::Name,     SortOrder::Desc) => "↓Name",
        (SortColumn::Size,     SortOrder::Asc)  => "↑Size",
        (SortColumn::Size,     SortOrder::Desc) => "↓Size",
        (SortColumn::Modified, SortOrder::Asc)  => "↑Date",
        (SortColumn::Modified, SortOrder::Desc) => "↓Date",
    };

    let hidden_marker = if panel.show_hidden { " [H]" } else { "" };

    let free = panel.disk_free.map(format_size).unwrap_or_default();
    let free_str = if free.is_empty() { String::new() } else { format!("  {free} free") };

    let sel_str = if panel.selected_names.is_empty() {
        String::new()
    } else {
        let total: u64 = panel.entries.iter()
            .filter(|e| panel.selected_names.contains(&e.name))
            .filter_map(|e| e.size)
            .sum();
        format!("  [{}  {}]", panel.selected_names.len(), format_size(total))
    };

    if focused {
        format!(" {path}  {sort_indicator}{hidden_marker}{free_str}{sel_str} ")
    } else {
        format!(" {path} ")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Column header row
// ─────────────────────────────────────────────────────────────────────────────

fn render_column_header(
    frame:    &mut Frame,
    layout:   &ColumnLayout,
    area:     Rect,
    sort_col: SortColumn,
    sort_ord: SortOrder,
    theme:    &Theme,
) {
    let hdr_style  = theme.col_header;
    let sort_style = theme.col_sorted;

    let arrow = match sort_ord {
        SortOrder::Asc  => "↑",
        SortOrder::Desc => "↓",
    };

    // Name column header
    let name_hdr = if sort_col == SortColumn::Name {
        format!("{arrow}Name")
    } else {
        "Name".into()
    };
    let name_style = if sort_col == SortColumn::Name { sort_style } else { hdr_style };

    let mut spans = vec![
        Span::raw(" "), // 1-char prefix placeholder
        Span::styled(format!("{:<width$}", name_hdr, width = layout.name_w), name_style),
    ];

    if layout.show_size {
        let s = if sort_col == SortColumn::Size {
            format!("{arrow}Size   ")
        } else {
            "Size   ".into()
        };
        let style = if sort_col == SortColumn::Size { sort_style } else { hdr_style };
        spans.push(Span::styled(format!("{:>8}", s), style));
    }
    if layout.show_date {
        let s = if sort_col == SortColumn::Modified {
            format!("{arrow}Date      ")
        } else {
            "Date      ".into()
        };
        let style = if sort_col == SortColumn::Modified { sort_style } else { hdr_style };
        spans.push(Span::styled(format!(" {:<10}", s), style));
    }
    if layout.show_perms {
        spans.push(Span::styled(" Perms    ", hdr_style));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry row builder
// ─────────────────────────────────────────────────────────────────────────────

fn build_list_item(
    entry:        &crate::vfs::VfsFileInfo,
    layout:       &ColumnLayout,
    is_cursor:    bool,
    is_marked:    bool,
    focused:      bool,
    quick_search: &str,
    theme:        &Theme,
) -> ListItem<'static> {
    // ── Base style for the entire row ─────────────────────────────────────
    let row_style = if is_cursor && focused {
        if is_marked { theme.entry_cursor_marked } else { theme.entry_cursor }
    } else if is_marked {
        theme.entry_marked
    } else {
        // Inactive-panel cursor: no background; name_style provides colour.
        Style::default()
    };

    // ── Name colour ───────────────────────────────────────────────────────
    let name_style = if is_cursor || is_marked {
        row_style
    } else if entry.is_symlink {
        theme.entry_symlink
    } else if entry.is_dir {
        theme.entry_dir
    } else {
        theme.file_kind_style(&entry.name, entry.is_executable)
    };

    // ── Type indicator prefix: '/' dir  '*' exec  ' ' regular file ──────
    let indicator = if entry.is_dir {
        "/"
    } else if entry.is_executable {
        "*"
    } else {
        " "
    };
    // The indicator takes the same colour as the name except on cursor/marked rows
    let indicator_style = if is_cursor || is_marked { row_style } else { name_style };

    // ── Name — with quick-search prefix highlighted ───────────────────────
    let mut spans: Vec<Span> = vec![Span::styled(indicator, indicator_style)];

    let name_truncated = truncate_str(&entry.name, layout.name_w);
    let qs_lower       = quick_search.to_ascii_lowercase();

    if !quick_search.is_empty() && name_truncated.to_ascii_lowercase().starts_with(&qs_lower) {
        let match_chars = quick_search.chars().count();
        let (matched, rest) = split_at_char(&name_truncated, match_chars);

        let hl_style = if is_cursor && focused {
            theme.entry_search_hl
        } else {
            theme.entry_search_hl_nc
        };

        spans.push(Span::styled(matched.to_owned(), hl_style));
        let rest_padded = format!(
            "{:<width$}",
            rest,
            width = layout.name_w.saturating_sub(match_chars)
        );
        spans.push(Span::styled(rest_padded, name_style));
    } else {
        spans.push(Span::styled(
            format!("{:<width$}", name_truncated, width = layout.name_w),
            name_style,
        ));
    }

    // ── Size column ───────────────────────────────────────────────────────
    if layout.show_size {
        let size_str = if entry.is_dir {
            "<DIR>   ".to_string()
        } else {
            entry.size.map(format_size).unwrap_or_default()
        };
        let size_style = if is_cursor || is_marked { row_style } else { theme.col_size };
        spans.push(Span::styled(
            format!("{:>8}", size_str),
            size_style,
        ));
    }

    // ── Date column ───────────────────────────────────────────────────────
    if layout.show_date {
        let date_str = entry
            .modified
            .map(|t| {
                let dt: DateTime<Local> = t.into();
                dt.format("%Y-%m-%d").to_string()
            })
            .unwrap_or_else(|| "          ".into());
        let date_style = if is_cursor || is_marked { row_style } else { theme.col_date };
        spans.push(Span::styled(format!(" {date_str}"), date_style));
    }

    // ── Permissions column ────────────────────────────────────────────────
    if layout.show_perms {
        let perm_style = if is_cursor || is_marked { row_style } else { theme.col_perms };
        spans.push(Span::styled(
            format!(" {:<9}", &entry.permissions),
            perm_style,
        ));
    }

    ListItem::new(Line::from(spans))
}

// ─────────────────────────────────────────────────────────────────────────────
// Quick-search bar
// ─────────────────────────────────────────────────────────────────────────────

fn render_search_bar(frame: &mut Frame, query: &str, area: Rect) {
    let label_style = Style::default().fg(Color::Black).bg(Color::Yellow);
    let query_style = Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let spans = vec![
        Span::styled(" Search: ", label_style),
        Span::styled(format!("{query}_"), query_style),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility functions
// ─────────────────────────────────────────────────────────────────────────────

pub fn format_size(bytes: u64) -> String {
    const K: u64 = 1_024;
    const M: u64 = K * 1_024;
    const G: u64 = M * 1_024;
    const T: u64 = G * 1_024;

    if bytes >= T      { format!("{:.1} T", bytes as f64 / T as f64) }
    else if bytes >= G { format!("{:.1} G", bytes as f64 / G as f64) }
    else if bytes >= M { format!("{:.1} M", bytes as f64 / M as f64) }
    else if bytes >= K { format!("{:.1} K", bytes as f64 / K as f64) }
    else               { format!("{bytes} B") }
}

/// Truncate `s` to at most `max_chars` characters (not bytes), appending `…`
/// if the string was actually truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_owned()
    } else if max_chars == 0 {
        String::new()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars.saturating_sub(1))
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

/// Split `s` after `n` characters (not bytes).  Returns `(prefix, rest)`.
fn split_at_char(s: &str, n: usize) -> (&str, &str) {
    let byte_idx = s
        .char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.split_at(byte_idx)
}
