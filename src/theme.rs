//! UI color schemes (themes).

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    #[default]
    Blue,
    DosNavigator,
}

impl ThemeKind {
    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Blue         => "Blue Classic",
            ThemeKind::DosNavigator => "Dos Navigator",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ThemeKind::Blue         => "Blue",
            ThemeKind::DosNavigator => "DosNavigator",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s {
            "DosNavigator" => ThemeKind::DosNavigator,
            _              => ThemeKind::Blue, // covers "Blue" and old "FarManager" saves
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub kind: ThemeKind,

    // ── Panels ────────────────────────────────────────────────────────────
    pub panel_border_active:   Style,
    pub panel_border_inactive: Style,
    pub panel_sep_active:      Style,
    pub panel_sep_inactive:    Style,

    // ── File entries ──────────────────────────────────────────────────────
    pub entry_dir:           Style,
    pub entry_file:          Style,
    pub entry_symlink:       Style,
    pub entry_exec:          Style,  // executable files (chmod +x)
    pub entry_archive:       Style,  // .zip .tar .gz .bz2 .7z .rar
    pub entry_image:         Style,  // .jpg .png .gif .webp .svg
    pub entry_media:         Style,  // .mp4 .avi .mp3 .flac
    pub entry_document:      Style,  // .pdf .doc .xls
    pub entry_data:          Style,  // .json .yaml .toml .xml .sql
    pub entry_code:          Style,  // .rs .py .js .c .go .sh
    pub entry_cursor:        Style,
    pub entry_cursor_marked: Style,
    pub entry_marked:        Style,
    /// Search highlight for the row under the cursor.
    pub entry_search_hl:     Style,
    /// Search highlight for rows that are NOT the cursor.
    pub entry_search_hl_nc:  Style,

    // ── Column headers / metadata ─────────────────────────────────────────
    pub col_header: Style,
    pub col_sorted: Style,
    pub col_size:   Style,
    pub col_date:   Style,
    pub col_perms:  Style,

    // ── Command line ──────────────────────────────────────────────────────
    pub cmdline_path: Style,
    pub cmdline_text: Style,

    // ── F-key status bar ─────────────────────────────────────────────────
    pub status_key:   Style,
    pub status_label: Style,
    pub status_sep:   Style,

    // ── Shell output view ─────────────────────────────────────────────────
    pub output_border:   Style,
    pub output_cmd_path: Style,
    pub output_cmd_text: Style,

    // ── Menu ─────────────────────────────────────────────────────────────
    pub menu_bar:           Style,
    pub menu_bar_active:    Style,
    pub menu_item:          Style,
    pub menu_item_selected: Style,
    pub menu_border:        Style,
    pub menu_shortcut:      Style,

    // ── Popups ────────────────────────────────────────────────────────────
    pub popup_border:       Style,
    pub popup_error_border: Style,
    pub popup_title:        Style,
    pub popup_text:         Style,
    pub popup_selected:     Style,
    pub popup_hint:         Style,
    pub popup_input_field:  Style,
    pub popup_input_cursor: Style,
    pub popup_gauge:        Style,
}

impl Theme {
    pub fn from_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Blue         => Self::blue_classic(),
            ThemeKind::DosNavigator => Self::dos_navigator(),
        }
    }

    /// Return the appropriate style for a plain file entry based on its name
    /// and whether it has the execute bit set.
    pub fn file_kind_style(&self, name: &str, is_exec: bool) -> Style {
        if is_exec {
            return self.entry_exec;
        }
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            // Archives
            "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" | "xz"
            | "tgz" | "tbz2" | "wpress" | "zst" | "lz4" => self.entry_archive,
            // Images
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp"
            | "svg" | "ico" | "tiff" | "tif" | "heic" | "avif" => self.entry_image,
            // Media
            "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm"
            | "mp3" | "flac" | "wav" | "ogg" | "aac" | "m4a" => self.entry_media,
            // Documents
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt"
            | "pptx" | "odt" | "ods" | "odp" | "epub" => self.entry_document,
            // Source code
            "rs" | "py" | "js" | "ts" | "go" | "c" | "cpp" | "h" | "hpp"
            | "java" | "rb" | "php" | "lua" | "sh" | "bash" | "zsh"
            | "fish" | "pl" | "r" | "swift" | "kt" | "cs" | "dart" => self.entry_code,
            // Data / config
            "json" | "yaml" | "yml" | "toml" | "xml" | "ini"
            | "conf" | "cfg" | "env" | "css" | "html" | "htm"
            | "sql" | "csv" | "tsv" | "lock" => self.entry_data,
            _ => self.entry_file,
        }
    }

    pub fn blue_classic() -> Self {
        Self {
            kind: ThemeKind::Blue,

            panel_border_active:   Style::default().fg(Color::Cyan),
            panel_border_inactive: Style::default().fg(Color::DarkGray),
            panel_sep_active:      Style::default().fg(Color::DarkGray),
            panel_sep_inactive:    Style::default().fg(Color::Black),

            entry_dir:           Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD),
            entry_file:          Style::default().fg(Color::White),
            entry_symlink:       Style::default().fg(Color::LightCyan),
            entry_exec:          Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            entry_archive:       Style::default().fg(Color::LightMagenta),
            entry_image:         Style::default().fg(Color::LightCyan),
            entry_media:         Style::default().fg(Color::Magenta),
            entry_document:      Style::default().fg(Color::Cyan),
            entry_data:          Style::default().fg(Color::Gray),
            entry_code:          Style::default().fg(Color::LightYellow),
            entry_cursor:        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
            entry_cursor_marked: Style::default().fg(Color::Yellow).bg(Color::Blue).add_modifier(Modifier::BOLD),
            entry_marked:        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            entry_search_hl:     Style::default().fg(Color::Yellow).bg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            entry_search_hl_nc:  Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),

            col_header: Style::default().fg(Color::DarkGray),
            col_sorted: Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            col_size:   Style::default().fg(Color::Gray),
            col_date:   Style::default().fg(Color::DarkGray),
            col_perms:  Style::default().fg(Color::DarkGray),

            cmdline_path: Style::default().fg(Color::DarkGray),
            cmdline_text: Style::default().fg(Color::White),

            status_key:   Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
            status_label: Style::default().fg(Color::White).bg(Color::DarkGray),
            status_sep:   Style::default().bg(Color::Black),

            output_border:   Style::default().fg(Color::DarkGray),
            output_cmd_path: Style::default().fg(Color::DarkGray),
            output_cmd_text: Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),

            menu_bar:           Style::default().fg(Color::Black).bg(Color::Cyan),
            menu_bar_active:    Style::default().fg(Color::Black).bg(Color::White),
            menu_item:          Style::default().fg(Color::Cyan).bg(Color::Blue),
            menu_item_selected: Style::default().fg(Color::Black).bg(Color::Cyan),
            menu_border:        Style::default().fg(Color::Cyan).bg(Color::Blue),
            menu_shortcut:      Style::default().fg(Color::DarkGray).bg(Color::Blue),

            popup_border:       Style::default().fg(Color::Cyan),
            popup_error_border: Style::default().fg(Color::Red),
            popup_title:        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            popup_text:         Style::default().fg(Color::White),
            popup_selected:     Style::default().fg(Color::Black).bg(Color::Cyan),
            popup_hint:         Style::default().fg(Color::DarkGray),
            popup_input_field:  Style::default().fg(Color::Black).bg(Color::White),
            popup_input_cursor: Style::default().fg(Color::White).bg(Color::DarkGray).add_modifier(Modifier::SLOW_BLINK),
            popup_gauge:        Style::default().fg(Color::Cyan).bg(Color::DarkGray),
        }
    }

    pub fn dos_navigator() -> Self {
        Self {
            kind: ThemeKind::DosNavigator,

            panel_border_active:   Style::default().fg(Color::Yellow),
            panel_border_inactive: Style::default().fg(Color::Green),
            panel_sep_active:      Style::default().fg(Color::Green),
            panel_sep_inactive:    Style::default().fg(Color::DarkGray),

            entry_dir:           Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            entry_file:          Style::default().fg(Color::White),
            entry_symlink:       Style::default().fg(Color::LightGreen),
            entry_exec:          Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            entry_archive:       Style::default().fg(Color::LightMagenta),
            entry_image:         Style::default().fg(Color::LightCyan),
            entry_media:         Style::default().fg(Color::Magenta),
            entry_document:      Style::default().fg(Color::Cyan),
            entry_data:          Style::default().fg(Color::Gray),
            entry_code:          Style::default().fg(Color::LightGreen),
            entry_cursor:        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
            entry_cursor_marked: Style::default().fg(Color::Yellow).bg(Color::Red).add_modifier(Modifier::BOLD),
            entry_marked:        Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD),
            entry_search_hl:     Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            entry_search_hl_nc:  Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),

            col_header: Style::default().fg(Color::Green),
            col_sorted: Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            col_size:   Style::default().fg(Color::Green),
            col_date:   Style::default().fg(Color::DarkGray),
            col_perms:  Style::default().fg(Color::DarkGray),

            cmdline_path: Style::default().fg(Color::Green),
            cmdline_text: Style::default().fg(Color::White),

            status_key:   Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
            status_label: Style::default().fg(Color::Black).bg(Color::White),
            status_sep:   Style::default().bg(Color::Black),

            output_border:   Style::default().fg(Color::Green),
            output_cmd_path: Style::default().fg(Color::Green),
            output_cmd_text: Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),

            menu_bar:           Style::default().fg(Color::Black).bg(Color::Green),
            menu_bar_active:    Style::default().fg(Color::Black).bg(Color::Yellow),
            menu_item:          Style::default().fg(Color::White).bg(Color::DarkGray),
            menu_item_selected: Style::default().fg(Color::Black).bg(Color::Yellow),
            menu_border:        Style::default().fg(Color::Yellow).bg(Color::DarkGray),
            menu_shortcut:      Style::default().fg(Color::Green).bg(Color::DarkGray),

            popup_border:       Style::default().fg(Color::Yellow),
            popup_error_border: Style::default().fg(Color::LightRed),
            popup_title:        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            popup_text:         Style::default().fg(Color::White),
            popup_selected:     Style::default().fg(Color::Black).bg(Color::Yellow),
            popup_hint:         Style::default().fg(Color::Green),
            popup_input_field:  Style::default().fg(Color::Black).bg(Color::White),
            popup_input_cursor: Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
            popup_gauge:        Style::default().fg(Color::Yellow).bg(Color::DarkGray),
        }
    }
}
