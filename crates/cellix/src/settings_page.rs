use cellix_render::{Bounds, Grid, Rect, Rgb, Style};

use crate::integrations;
use crate::settings::default_server;
use crate::theme::Theme;

const SECTIONS: [(&str, Section); 4] = [
    ("Appearance", Section::Appearance),
    ("Editor", Section::Editor),
    ("Integrations", Section::Integrations),
    ("Keyboard", Section::Keyboard),
];
const THEMES: [(&str, &str, &str); 3] = [
    ("auto", "Follow terminal", "Matches your terminal"),
    ("dark", "Dark", "Always dark"),
    ("light", "Light", "Always light"),
];
const KEYMAPS: [(&str, &str); 3] = [("simple", "Standard"), ("vim", "Vim"), ("neovim", "Neovim")];
const LANGUAGES: [&str; 8] = [
    "Rust",
    "Python",
    "Go",
    "TypeScript",
    "Terraform",
    "YAML",
    "TOML",
    "C",
];
/// Real keys, so this list stays a promise. Cmd stands in for Ctrl where the terminal
/// passes it on.
const SHORTCUTS: [(&str, &str); 14] = [
    ("Ctrl+P", "Open a file"),
    ("Ctrl+Shift+P", "Commands"),
    ("Ctrl+G", "Review changes"),
    ("Ctrl+Shift+F", "Search the project"),
    ("Ctrl+F", "Find in file"),
    ("Ctrl+D", "Select the next match"),
    ("Ctrl+/", "Toggle comment"),
    ("Ctrl+S", "Save"),
    ("Ctrl+W", "Close tab"),
    ("Ctrl+B", "Toggle sidebar"),
    ("Ctrl+\\", "Split pane"),
    ("Ctrl+Space", "Complete"),
    ("F12", "Go to definition"),
    ("F2", "Rename symbol"),
];
/// The list of sections, when there's room to keep it beside the page.
const NAV_WIDTH: u16 = 20;
/// Narrower than this, the sections run across the top instead.
const SIDE_NAV: u16 = 76;
const MAX_WIDTH: u16 = 100;
const CARD_WIDTH: u16 = 24;
const CARD_HEIGHT: u16 = 8;
const PREVIEW_ROWS: u16 = 6;
const SEGMENT_WIDTH: u16 = 11;
const BUTTON_WIDTH: u16 = 16;
const TOGGLE_WIDTH: f32 = 2.2;
const TITLE_SIZE: u8 = 170;
const NOTE_SIZE: u8 = 90;
const RADIUS: u8 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Editor,
    Integrations,
    Keyboard,
}

/// Anything on the page that takes a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Section(Section),
    Theme(&'static str),
    SmoothScroll,
    Minimap,
    Keymap(&'static str),
    ClaudeCode,
    Ghostty,
}

/// The settings as they stand, passed in so the page holds no second copy.
pub struct Values<'a> {
    pub theme: &'a str,
    pub keymap: &'a str,
    pub smooth_scroll: bool,
    pub minimap: bool,
}

/// What was found on this machine. Looking touches the disk, so it happens on opening and
/// after a change rather than every frame.
#[derive(Default)]
pub struct Found {
    pub claude: bool,
    pub claude_installed: bool,
    pub codex: bool,
    /// Whether Ghostty keeps Cmd+D, when this is Ghostty.
    pub ghostty_keeps_cmd_d: Option<bool>,
    /// Language, server and whether it's installed.
    pub servers: Vec<(&'static str, String, bool)>,
}

impl Found {
    pub fn look() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self {
            claude: integrations::on_path("claude", var),
            claude_installed: integrations::claude_installed(var),
            codex: integrations::on_path("codex", var),
            ghostty_keeps_cmd_d: integrations::ghostty_keeps_cmd_d(var),
            servers: LANGUAGES
                .iter()
                .filter_map(|language| {
                    let server = default_server(language)?.into_iter().next()?;
                    let found = integrations::on_path(&server, var);
                    Some((*language, server, found))
                })
                .collect(),
        }
    }
}

/// The Settings page, which stands where the file would.
pub struct SettingsPage {
    pub section: Section,
    /// The control the keyboard is on, as an index into `controls`.
    pub focus: Option<usize>,
    pub found: Found,
    /// What the last action did.
    pub note: String,
}

impl SettingsPage {
    pub fn new(found: Found) -> Self {
        Self {
            section: Section::Appearance,
            focus: None,
            found,
            note: String::new(),
        }
    }

    /// Where the page's content goes, under the tabs.
    fn content(area: Rect) -> Rect {
        let side = if area.w >= SIDE_NAV { NAV_WIDTH + 2 } else { 0 };
        let top = area.y + if side > 0 { 3 } else { 4 };
        let left = area.x + side + 2;
        let width = area.right().saturating_sub(left + 2).min(MAX_WIDTH);
        Rect::new(left, top, width, area.bottom().saturating_sub(top))
    }

    /// Every control and where it is. Drawing and clicking share this, so a click always
    /// lands on what's shown.
    pub fn controls(&self, area: Rect) -> Vec<(Rect, Control)> {
        let mut spots = Vec::new();
        for (i, (name, section)) in SECTIONS.iter().enumerate() {
            let rect = if area.w >= SIDE_NAV {
                Rect::new(area.x + 2, area.y + 3 + i as u16, NAV_WIDTH, 1)
            } else {
                let before: usize = SECTIONS[..i].iter().map(|s| s.0.len() + 3).sum();
                let width = name.len() as u16 + 2;
                Rect::new(area.x + 2 + before as u16, area.y + 2, width, 1)
            };
            spots.push((rect, Control::Section(*section)));
        }
        let page = Self::content(area);
        match self.section {
            Section::Appearance => {
                let card = CARD_WIDTH.min(page.w.saturating_sub(4) / 3);
                for (i, (name, _, _)) in THEMES.iter().enumerate() {
                    let x = page.x + i as u16 * (card + 2);
                    let rect = Rect::new(x, page.y + 3, card, CARD_HEIGHT);
                    spots.push((rect, Control::Theme(name)));
                }
                let below = page.y + 3 + CARD_HEIGHT + 2;
                spots.push((Rect::new(page.x, below, page.w, 2), Control::SmoothScroll));
                spots.push((Rect::new(page.x, below + 3, page.w, 2), Control::Minimap));
            }
            Section::Editor => {
                let start = page.right().saturating_sub(SEGMENT_WIDTH * 3);
                for (i, (name, _)) in KEYMAPS.iter().enumerate() {
                    let x = start + i as u16 * SEGMENT_WIDTH;
                    let rect = Rect::new(x, page.y + 3, SEGMENT_WIDTH, 1);
                    spots.push((rect, Control::Keymap(name)));
                }
            }
            Section::Integrations => {
                let x = page.right().saturating_sub(BUTTON_WIDTH + 1);
                let row = |index: u16| page.y + 4 + index * 5;
                spots.push((Rect::new(x, row(0), BUTTON_WIDTH, 1), Control::ClaudeCode));
                if self.found.ghostty_keeps_cmd_d == Some(true) {
                    let rect = Rect::new(x, row(2), BUTTON_WIDTH, 1);
                    spots.push((rect, Control::Ghostty));
                }
            }
            Section::Keyboard => {}
        }
        spots
    }

    pub fn control_at(&self, area: Rect, x: u16, y: u16) -> Option<Control> {
        let spots = self.controls(area);
        spots
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, c)| *c)
    }

    pub fn step(&mut self, area: Rect, delta: isize) {
        let count = self.controls(area).len() as isize;
        let at = self.focus.map_or(-1, |f| f as isize);
        self.focus = Some((at + delta).rem_euclid(count.max(1)) as usize);
    }

    pub fn focused(&self, area: Rect) -> Option<Control> {
        Some(self.controls(area).get(self.focus?)?.1)
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        values: &Values,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        grid.fill(area, theme.text);
        let spots = self.controls(area);
        let hovered = |control: Control| {
            let spot = spots.iter().find(|(_, c)| *c == control);
            spot.is_some_and(|(r, _)| hover.is_some_and(|h| r.contains(h.0, h.1)))
        };
        for (rect, control) in &spots {
            if let Control::Section(section) = control {
                self.draw_section_tab(grid, *rect, *section, hovered(*control), theme);
            }
        }
        let page = Self::content(area);
        match self.section {
            Section::Appearance => {
                heading(
                    grid,
                    page,
                    "Appearance",
                    "Changes apply as you click.",
                    theme,
                );
                for (rect, control) in &spots {
                    if let Control::Theme(name) = control {
                        let on = values.theme == *name;
                        draw_card(grid, *rect, name, (on, hovered(*control)), theme);
                    }
                }
                let below = page.y + 3 + CARD_HEIGHT + 2;
                let smooth = (
                    "Smooth scrolling",
                    "The wheel moves by pixels. Turn this off on a slow link.",
                );
                draw_toggle(grid, page, below, smooth, values.smooth_scroll, theme);
                let minimap = ("Minimap", "The file at a glance, beside the text.");
                draw_toggle(grid, page, below + 3, minimap, values.minimap, theme);
            }
            Section::Editor => self.draw_editor(grid, page, &spots, values, theme),
            Section::Integrations => self.draw_integrations(grid, page, hover, theme),
            Section::Keyboard => draw_keyboard(grid, page, theme),
        }
        if let Some((rect, _)) = self.focus.and_then(|f| spots.get(f)) {
            let ring = Bounds::inset(*rect, -0.15, -0.1);
            grid.panel(ring, RADIUS, None, Some(theme.focus));
        }
    }

    fn draw_section_tab(
        &self,
        grid: &mut Grid,
        rect: Rect,
        section: Section,
        hovered: bool,
        theme: &Theme,
    ) {
        let on = self.section == section;
        let name = SECTIONS.iter().find(|s| s.1 == section).map_or("", |s| s.0);
        let wash = if on {
            Some(theme.tint)
        } else {
            hovered.then_some(theme.row_hover)
        };
        if let Some(fill) = wash {
            grid.panel(Bounds::inset(rect, 0.0, 0.06), 22, Some(fill), None);
        }
        let style = Style {
            fg: if on { theme.accent } else { theme.text.fg },
            ..theme.text
        };
        let x = rect.x as f32 + 1.0;
        grid.label_at(x, rect.x + 1, rect.y, name, style, rect.right(), 100);
    }

    fn draw_editor(
        &self,
        grid: &mut Grid,
        page: Rect,
        spots: &[(Rect, Control)],
        values: &Values,
        theme: &Theme,
    ) {
        heading(grid, page, "Editor", "How the text behaves.", theme);
        draw_text(
            grid,
            page,
            page.y + 3,
            ("Keymap", "Standard works as in VS Code."),
            theme,
        );
        for (rect, control) in spots {
            if let Control::Keymap(name) = control {
                let label = KEYMAPS.iter().find(|k| k.0 == *name).map_or("", |k| k.1);
                let on = values.keymap == *name;
                let fill = if on { theme.button } else { theme.tab.bg };
                let style = Style {
                    fg: if on { theme.button_text } else { theme.text.fg },
                    ..theme.text
                };
                let cells = format!("{label:^width$}", width = rect.w as usize);
                grid.button(rect.x, rect.y, &cells, label, fill, style);
            }
        }
        let mut y = page.y + 7;
        draw_note(grid, page, y, "LANGUAGE SERVERS", theme);
        y += 1;
        for (language, server, found) in &self.found.servers {
            if y >= page.bottom() {
                break;
            }
            let name = Style {
                bold: true,
                ..theme.text
            };
            grid.label(page.x, y, language, name, page.x + 16);
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.text
            };
            grid.label(
                page.x + 16,
                y,
                server,
                quiet,
                page.right().saturating_sub(16),
            );
            let (word, color) = if *found {
                ("Found", theme.git_added)
            } else {
                ("Not installed", theme.gutter.fg)
            };
            badge(grid, page.right().saturating_sub(13), y, word, color, theme);
            y += 1;
        }
    }

    fn draw_integrations(
        &self,
        grid: &mut Grid,
        page: Rect,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        heading(
            grid,
            page,
            "Integrations",
            "Cellix works inside your agents. Nothing here changes until you click.",
            theme,
        );
        let found = &self.found;
        let (word, color, detail, action) = match (found.claude, found.claude_installed) {
            (_, true) => (
                "Installed",
                theme.git_added,
                "Opens as a pane in every Claude session, with no flags to remember.",
                "Remove",
            ),
            (true, false) => (
                "Not installed",
                theme.gutter.fg,
                "Install to open Cellix as a pane in every Claude session.",
                "Install",
            ),
            (false, false) => (
                "Claude Code is not on this machine",
                theme.gutter.fg,
                "You can install now, and it's ready when Claude Code is.",
                "Install",
            ),
        };
        let spot = Rect::new(
            page.right().saturating_sub(BUTTON_WIDTH + 1),
            page.y + 4,
            BUTTON_WIDTH,
            1,
        );
        let primary = !found.claude_installed;
        draw_block(grid, page, 0, ("Claude Code", word, color), detail, theme);
        draw_button(grid, spot, action, primary, hover, theme);

        let (word, detail) = if found.codex {
            (
                "Found",
                "Cellix opens Codex. Its plugin isn't built yet, so it has no review pane.",
            )
        } else {
            (
                "Codex is not on this machine",
                "Cellix opens Codex once it's installed. Its plugin isn't built yet.",
            )
        };
        let color = if found.codex {
            theme.git_added
        } else {
            theme.gutter.fg
        };
        draw_block(grid, page, 1, ("Codex", word, color), detail, theme);

        match found.ghostty_keeps_cmd_d {
            Some(true) => {
                let title = ("Ghostty", "Cmd+D opens a split", theme.attention);
                let detail = "Ghostty keeps Cmd+D, so Cellix never sees it. Ctrl+D always works.";
                draw_block(grid, page, 2, title, detail, theme);
                let spot = Rect::new(spot.x, page.y + 14, BUTTON_WIDTH, 1);
                draw_button(grid, spot, "Free Cmd+D", true, hover, theme);
            }
            Some(false) => {
                let title = ("Ghostty", "Cmd+D is free", theme.git_added);
                let detail = "Cmd+D selects the next match. Reload Ghostty's config if it doesn't.";
                draw_block(grid, page, 2, title, detail, theme);
            }
            None => {}
        }
        if !self.note.is_empty() {
            let style = Style {
                fg: theme.accent,
                ..theme.text
            };
            let y = page.y + 19;
            grid.label(page.x, y, &self.note, style, page.right());
        }
    }
}

fn heading(grid: &mut Grid, page: Rect, title: &str, note: &str, theme: &Theme) {
    let style = Style {
        fg: theme.heading,
        bold: true,
        ..theme.text
    };
    let x = page.x as f32;
    grid.label_at(x, page.x, page.y, title, style, page.right(), TITLE_SIZE);
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    grid.label(page.x, page.y + 1, note, quiet, page.right());
}

fn draw_note(grid: &mut Grid, page: Rect, y: u16, text: &str, theme: &Theme) {
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    let x = page.x as f32;
    grid.label_at(x, page.x, y, text, quiet, page.right(), NOTE_SIZE - 10);
}

/// A title with a line under it, at the left of a row of two.
fn draw_text(grid: &mut Grid, page: Rect, y: u16, (title, detail): (&str, &str), theme: &Theme) {
    let name = Style {
        bold: true,
        ..theme.text
    };
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
    grid.label(page.x, y, title, name, room);
    grid.label(page.x, y + 1, detail, quiet, room);
}

fn draw_toggle(grid: &mut Grid, page: Rect, y: u16, text: (&str, &str), on: bool, theme: &Theme) {
    let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
    grid.hairline(above, theme.border);
    draw_text(grid, page, y, text, theme);
    let right = page.right() as f32;
    let x = right - TOGGLE_WIDTH;
    let track = if on { theme.button } else { theme.popup_border };
    let bounds = Bounds::cells(x, y as f32 + 0.2, TOGGLE_WIDTH, 0.6);
    grid.panel(bounds, RADIUS, Some(track), None);
    let knob_x = if on { right - 1.0 } else { x + 0.15 };
    let knob_color = if on { theme.button_text } else { theme.text.fg };
    let knob = Bounds::cells(knob_x, y as f32 + 0.28, 0.85, 0.44);
    grid.panel(knob, 22, Some(knob_color), None);
    let hidden = Style {
        painted: true,
        ..theme.text
    };
    let word = if on { "[on]" } else { "[off]" };
    let cell = page.right().saturating_sub(word.len() as u16);
    grid.put_str(cell, y, word, hidden, page.right());
}

/// The colours of a theme, as a tiny window.
fn draw_card(grid: &mut Grid, rect: Rect, name: &str, (on, hovered): (bool, bool), theme: &Theme) {
    let (_, title, detail) = THEMES
        .iter()
        .find(|t| t.0 == name)
        .copied()
        .unwrap_or(THEMES[0]);
    let sample = match name {
        "dark" => Theme::default(),
        "light" => Theme::named(Some("light")),
        _ => *theme,
    };
    let (x, y, w) = (rect.x as f32, rect.y as f32, rect.w as f32);
    let rows = PREVIEW_ROWS as f32;
    grid.panel(
        Bounds::cells(x, y, w, rows),
        RADIUS,
        Some(sample.activity.bg),
        None,
    );
    let inner = |left: f32, width: f32| Bounds::cells(left, y + 0.4, width, rows - 0.8);
    grid.panel(inner(x + 1.4, w * 0.26), 12, Some(sample.sidebar.bg), None);
    let sheet_x = x + 1.4 + w * 0.26 + 0.4;
    let sheet_w = x + w - 0.8 - sheet_x;
    grid.panel(inner(sheet_x, sheet_w), 12, Some(sample.text.bg), None);
    let line = |row: f32, width: f32, color: Rgb| {
        let bounds = Bounds::cells(sheet_x + 0.8, y + row, sheet_w * width, 0.2);
        (bounds, color)
    };
    for (bounds, color) in [
        line(1.1, 0.5, sample.accent),
        line(1.9, 0.75, sample.gutter.fg),
        line(2.7, 0.6, sample.gutter.fg),
        line(3.5, 0.7, sample.gutter.fg),
    ] {
        grid.panel(bounds, 6, Some(color), None);
    }
    let edge = if on {
        Some(theme.accent)
    } else if hovered {
        Some(theme.popup_border)
    } else {
        Some(theme.border)
    };
    grid.panel(Bounds::cells(x, y, w, rows), RADIUS, None, edge);
    let label_y = rect.y + PREVIEW_ROWS;
    let style = Style {
        bold: on,
        fg: if on { theme.accent } else { theme.text.fg },
        ..theme.text
    };
    grid.label(rect.x, label_y, title, style, rect.right());
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    let at = x;
    grid.label_at(
        at,
        rect.x,
        label_y + 1,
        detail,
        quiet,
        rect.right(),
        NOTE_SIZE,
    );
}

fn badge(grid: &mut Grid, x: u16, y: u16, word: &str, color: Rgb, theme: &Theme) {
    let dot = Bounds::cells(x as f32, y as f32 + 0.36, 0.42, 0.28);
    grid.panel(dot, 14, Some(color), None);
    let style = Style {
        fg: color,
        ..theme.text
    };
    let text_x = x as f32 + 1.0;
    grid.label_at(text_x, x + 1, y, word, style, grid.width(), NOTE_SIZE);
}

/// One integration: a name, its state and a line about it, in a raised box.
fn draw_block(
    grid: &mut Grid,
    page: Rect,
    index: u16,
    (name, word, color): (&str, &str, Rgb),
    detail: &str,
    theme: &Theme,
) {
    let top = page.y + 3 + index * 5;
    let box_area = Rect::new(page.x, top, page.w, 4);
    let surface = Bounds::inset(box_area, 0.0, 0.0);
    grid.panel(surface, RADIUS, Some(theme.tab.bg), Some(theme.border));
    let style = Style {
        fg: theme.heading,
        bold: true,
        bg: theme.tab.bg,
        ..theme.text
    };
    let x = page.x + 2;
    let end = grid.label(x, top + 1, name, style, page.right());
    badge(grid, end + 1, top + 1, word, color, theme);
    let quiet = Style {
        fg: theme.gutter.fg,
        bg: theme.tab.bg,
        ..theme.text
    };
    let room = page.right().saturating_sub(BUTTON_WIDTH + 2);
    grid.label(x, top + 2, detail, quiet, room);
}

fn draw_button(
    grid: &mut Grid,
    rect: Rect,
    label: &str,
    primary: bool,
    hover: Option<(u16, u16)>,
    theme: &Theme,
) {
    let hovered = hover.is_some_and(|h| rect.contains(h.0, h.1));
    let fill = match (primary, hovered) {
        (true, _) => theme.button,
        (false, true) => theme.pick,
        (false, false) => theme.row_hover,
    };
    let fg = if primary {
        theme.button_text
    } else {
        theme.text.fg
    };
    let style = Style { fg, ..theme.text };
    let cells = format!("{label:^width$}", width = rect.w as usize);
    grid.button(rect.x, rect.y, &cells, label, fill, style);
}

fn draw_keyboard(grid: &mut Grid, page: Rect, theme: &Theme) {
    heading(
        grid,
        page,
        "Keyboard",
        "Cmd works in place of Ctrl wherever your terminal passes it on.",
        theme,
    );
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    let column = (page.w / 2).max(30);
    for (i, (keys, what)) in SHORTCUTS.iter().enumerate() {
        let half = SHORTCUTS.len() / 2;
        let (x, row) = if i < half {
            (page.x, i)
        } else {
            (page.x + column, i - half)
        };
        let y = page.y + 3 + row as u16;
        if y >= page.bottom() {
            continue;
        }
        grid.label(x, y, keys, theme.text, x + 14);
        grid.label(x + 14, y, what, quiet, (x + column).min(page.right()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 33,
        y: 0,
        w: 120,
        h: 40,
    };

    fn page(section: Section) -> SettingsPage {
        SettingsPage {
            section,
            focus: None,
            found: Found::default(),
            note: String::new(),
        }
    }

    #[test]
    fn every_control_can_be_reached_by_both_mouse_and_keys() {
        let mut settings = page(Section::Appearance);
        let spots = settings.controls(AREA);
        assert!(spots.contains(&(spots[4].0, Control::Theme("auto"))));
        for (rect, control) in &spots {
            assert_eq!(settings.control_at(AREA, rect.x, rect.y), Some(*control));
        }
        for _ in 0..spots.len() {
            settings.step(AREA, 1);
        }
        assert_eq!(settings.focused(AREA), spots.last().map(|s| s.1));
        settings.step(AREA, 1);
        assert_eq!(settings.focused(AREA), Some(spots[0].1), "and it wraps");
    }

    #[test]
    fn nothing_overlaps_in_a_narrow_pane() {
        let narrow = Rect::new(0, 0, 60, 30);
        for (_, section) in SECTIONS {
            let spots = page(section).controls(narrow);
            for (i, (a, _)) in spots.iter().enumerate() {
                assert!(a.right() <= narrow.right(), "{a:?}");
                for (b, _) in &spots[i + 1..] {
                    let apart = a.right() <= b.x
                        || b.right() <= a.x
                        || a.bottom() <= b.y
                        || b.bottom() <= a.y;
                    assert!(apart, "{a:?} and {b:?}");
                }
            }
        }
    }

    #[test]
    fn a_button_only_shows_when_it_has_something_to_do() {
        let mut settings = page(Section::Integrations);
        let free = |s: &SettingsPage| s.controls(AREA).iter().any(|c| c.1 == Control::Ghostty);
        assert!(!free(&settings), "not in other terminals");
        settings.found.ghostty_keeps_cmd_d = Some(true);
        assert!(free(&settings));
        settings.found.ghostty_keeps_cmd_d = Some(false);
        assert!(!free(&settings), "not once it's done");
    }

    #[test]
    fn the_shortcuts_listed_are_real() {
        use cellix_input::{Key, KeyEvent, Mods, simple_keymap};
        for (keys, _) in SHORTCUTS {
            let shift = keys.contains("Shift");
            let event = match keys.rsplit('+').next().unwrap() {
                "Space" => KeyEvent::ctrl(' '),
                "F12" => KeyEvent::plain(Key::F(12)),
                "F2" => KeyEvent::plain(Key::F(2)),
                letter => KeyEvent {
                    key: Key::Char(letter.chars().next().unwrap().to_ascii_lowercase()),
                    mods: Mods {
                        ctrl: true,
                        shift,
                        ..Mods::default()
                    },
                },
            };
            assert!(simple_keymap(event).is_some(), "{keys}");
        }
    }
}
