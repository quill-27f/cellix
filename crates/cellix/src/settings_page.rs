use std::sync::mpsc::Receiver;

use cellix_render::{Bounds, Font, Grid, Measure, Rect, Rgb, Style};

use crate::integrations;
use crate::settings::{SERVERS, Server};
use crate::theme::Theme;

/// Rows from one integration's card to the next.
const BLOCK_STEP: u16 = 4;

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
const BUTTON_WIDTH: u16 = 12;
const TOGGLE_WIDTH: f32 = 2.6;
const TITLE_SIZE: u8 = 170;
const NOTE_SIZE: u8 = 90;
const RADIUS: u8 = 30;
/// Where the servers' rows start, from the top of the page.
const SERVERS_ROW: u16 = 7;
const INSTALL_WIDTH: u16 = 9;
/// Columns from an integration's name to its state.
const STATUS_GAP: f32 = 1.1;

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
    Codex,
    Gemini,
    Ghostty,
    /// A language server to install, as an index into `SERVERS`.
    InstallServer(usize),
    /// A command to put on the clipboard.
    CopyInstall(&'static str),
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
    pub codex_installed: bool,
    pub gemini: bool,
    pub gemini_installed: bool,
    /// Whether Ghostty keeps Cmd+D, when this is Ghostty.
    pub ghostty_keeps_cmd_d: Option<bool>,
    /// For each of `SERVERS`, whether it's installed.
    pub servers: Vec<bool>,
}

impl Found {
    pub fn look() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self {
            claude: integrations::on_path("claude", var),
            claude_installed: integrations::claude_installed(var),
            codex: integrations::on_path("codex", var),
            codex_installed: integrations::codex_installed(var),
            gemini: integrations::on_path("gemini", var),
            gemini_installed: integrations::gemini_installed(var),
            ghostty_keeps_cmd_d: integrations::ghostty_keeps_cmd_d(var),
            servers: SERVERS
                .iter()
                .map(|s| integrations::on_path(s.command[0], var))
                .collect(),
        }
    }
}

/// One card on the Integrations page.
struct Block {
    name: &'static str,
    state: &'static str,
    /// How the state reads.
    tone: Tone,
    detail: &'static str,
    /// A button's label, whether it's the main one, and what it does.
    action: Option<(&'static str, bool, Control)>,
}

#[derive(Clone, Copy)]
enum Tone {
    Good,
    Quiet,
}

/// The Settings page, which stands where the file would.
pub struct SettingsPage {
    pub section: Section,
    /// The control the keyboard is on, as an index into `controls`.
    pub focus: Option<usize>,
    pub found: Found,
    /// What the last action did.
    pub note: String,
    /// The first language server in view.
    pub top: usize,
    /// An install under way, which says how it went.
    pub installing: Option<Receiver<String>>,
}

impl SettingsPage {
    pub fn new(found: Found) -> Self {
        Self {
            section: Section::Appearance,
            focus: None,
            found,
            note: String::new(),
            top: 0,
            installing: None,
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = SERVERS.len().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last) as usize;
    }

    /// The cards to show, which depend on what's on this machine.
    fn blocks(&self) -> Vec<Block> {
        let found = &self.found;
        let mut blocks = vec![Self::claude_block(found)];
        let agent = |name, (missing, install), on_path, installed, control, detail| {
            let (state, tone, detail, action) = match (on_path, installed) {
                (_, true) => (
                    "Installed",
                    Tone::Good,
                    "Shows its work in Agents, and hands it your comments.",
                    Some(("Remove", false, control)),
                ),
                (true, false) => (
                    "Plugin not installed",
                    Tone::Quiet,
                    detail,
                    Some(("Install", true, control)),
                ),
                (false, false) => (
                    missing,
                    Tone::Quiet,
                    "Install it first, then come back here.",
                    Some(("Copy install", false, Control::CopyInstall(install))),
                ),
            };
            Block {
                name,
                state,
                tone,
                detail,
                action,
            }
        };
        blocks.push(agent(
            "Codex",
            ("Codex not found", "npm install -g @openai/codex"),
            found.codex,
            found.codex_installed,
            Control::Codex,
            "Shows its work in Agents, and hands it your comments.",
        ));
        blocks.push(agent(
            "Gemini CLI",
            ("Gemini CLI not found", "npm install -g @google/gemini-cli"),
            found.gemini,
            found.gemini_installed,
            Control::Gemini,
            "Shows its work in Agents, and hands it your comments.",
        ));
        match found.ghostty_keeps_cmd_d {
            Some(true) => blocks.push(Block {
                name: "Ghostty keys",
                state: "Cmd+D goes to Ghostty",
                tone: Tone::Quiet,
                detail: "Ghostty keeps Cmd+D for itself. Ctrl+D always works.",
                action: Some(("Free Cmd+D", false, Control::Ghostty)),
            }),
            Some(false) => blocks.push(Block {
                name: "Ghostty keys",
                state: "Cmd+D is free",
                tone: Tone::Good,
                detail: "Reload Ghostty's config if Cmd+D doesn't select yet.",
                action: None,
            }),
            None => {}
        }
        blocks
    }

    fn claude_block(found: &Found) -> Block {
        let (state, tone, detail, action) = match (found.claude, found.claude_installed) {
            (_, true) => (
                "Installed",
                Tone::Good,
                "Opens as a pane in every Claude session, with no flags.",
                ("Remove", false),
            ),
            (true, false) => (
                "Plugin not installed",
                Tone::Quiet,
                "Opens Cellix as a pane in every Claude session.",
                ("Install", true),
            ),
            (false, false) => (
                "Claude Code not found",
                Tone::Quiet,
                "You can install now, and it's ready when Claude Code is.",
                ("Install", true),
            ),
        };
        Block {
            name: "Claude Code",
            state,
            tone,
            detail,
            action: Some((action.0, action.1, Control::ClaudeCode)),
        }
    }

    /// Where the page's content goes, under the tabs.
    fn content(area: Rect) -> Rect {
        let side = if area.w >= SIDE_NAV { NAV_WIDTH + 2 } else { 0 };
        let top = area.y + if side > 0 { 2 } else { 4 };
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
                Rect::new(area.x + 2, area.y + 2 + i as u16, NAV_WIDTH, 1)
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
                for (row, index) in self.server_rows(page).into_iter().enumerate() {
                    if !self.found.servers.get(index).copied().unwrap_or(false) {
                        let x = page.right().saturating_sub(INSTALL_WIDTH);
                        let y = page.y + SERVERS_ROW + row as u16;
                        let rect = Rect::new(x, y, INSTALL_WIDTH, 1);
                        spots.push((rect, Control::InstallServer(index)));
                    }
                }
            }
            Section::Integrations => {
                let x = page.right().saturating_sub(BUTTON_WIDTH + 2);
                for (index, block) in self.blocks().iter().enumerate() {
                    if let Some((_, _, control)) = block.action {
                        let y = page.y + 3 + index as u16 * BLOCK_STEP;
                        spots.push((Rect::new(x, y, BUTTON_WIDTH, 2), control));
                    }
                }
            }
            Section::Keyboard => {}
        }
        spots
    }

    /// The servers that fit on the page, by their place in `SERVERS`.
    fn server_rows(&self, page: Rect) -> Vec<usize> {
        let room = page.h.saturating_sub(SERVERS_ROW) as usize;
        let here = |index: &usize| self.found.servers.get(*index).copied().unwrap_or(false);
        // What you already have comes first.
        let mut order: Vec<usize> = (0..SERVERS.len()).collect();
        order.sort_by_key(|index| !here(index));
        order.into_iter().skip(self.top).take(room).collect()
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
        (hover, measure): (Option<(u16, u16)>, &dyn Measure),
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
                let minimap = ("Minimap", "The file at a glance, when the window has room.");
                draw_toggle(grid, page, below + 3, minimap, values.minimap, theme);
            }
            Section::Editor => self.draw_editor(grid, page, &spots, values, hover, theme),
            Section::Integrations => self.draw_integrations(grid, page, (hover, measure), theme),
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
            hovered.then_some(theme.page_hover)
        };
        if let Some(fill) = wash {
            grid.panel(Bounds::inset(rect, 0.0, 0.055), 18, Some(fill), None);
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
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        heading(grid, page, "Editor", "How keys and languages work.", theme);
        let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
        let keymap = ("Keymap", "Standard works as in VS Code.");
        draw_text(grid, page, (page.y + 3, room), keymap, theme);
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
                grid.large_button(rect.x, rect.y, &cells, label, fill, style);
            }
        }
        let line_y = page.y + SERVERS_ROW - 1;
        let above = Bounds::cells(page.x as f32, line_y as f32 - 0.5, page.w as f32, 0.0);
        grid.hairline(above, theme.border);
        let count = self.found.servers.iter().filter(|found| **found).count();
        let title = format!("LANGUAGE SERVERS, {count} OF {} FOUND", SERVERS.len());
        draw_note(grid, page, line_y, &title, theme);
        for (row, index) in self.server_rows(page).into_iter().enumerate() {
            let y = page.y + SERVERS_ROW + row as u16;
            self.draw_server(grid, page, (y, index), spots, hover, theme);
        }
    }

    /// One server: its languages and program, and whether it's here or a click from it.
    fn draw_server(
        &self,
        grid: &mut Grid,
        page: Rect,
        (y, index): (u16, usize),
        spots: &[(Rect, Control)],
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        let server: &Server = &SERVERS[index];
        let row = Rect::new(page.x, y, page.w, 1);
        let hovered = hover.is_some_and(|h| row.contains(h.0, h.1));
        if hovered {
            let wash = Bounds::cells(
                page.x as f32 - 0.6,
                y as f32 + 0.055,
                page.w as f32 + 1.2,
                0.89,
            );
            grid.panel(wash, 18, Some(theme.page_hover), None);
        }
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        grid.label(page.x, y, server.languages[0], theme.text, page.x + 16);
        let found = self.found.servers.get(index).copied().unwrap_or(false);
        // Hovering a missing server shows what its button will run.
        let end = page.right().saturating_sub(INSTALL_WIDTH + 1);
        if hovered && !found {
            let runs = format!("Runs {}", server.install);
            grid.label(page.x + 16, y, &runs, theme.text, end);
        } else {
            grid.label(page.x + 16, y, server.command[0], quiet, end);
        }
        if found {
            let state = Style {
                fg: theme.git_added,
                ..theme.text
            };
            grid.label_right(page.right(), y, "Found", state, NOTE_SIZE);
            return;
        }
        let spot = spots
            .iter()
            .find(|(_, c)| *c == Control::InstallServer(index));
        if let Some((rect, _)) = spot {
            // Quiet until wanted: twenty filled buttons in a column would drown the list.
            if hovered {
                let style = Style {
                    fg: theme.button_text,
                    ..theme.text
                };
                let cells = format!("{:^width$}", "Install", width = rect.w as usize);
                grid.button(rect.x, rect.y, &cells, "Install", theme.button, style);
            } else {
                let style = Style {
                    fg: theme.accent,
                    ..theme.text
                };
                grid.label_right(page.right(), y, "Install", style, NOTE_SIZE);
            }
        }
    }

    fn draw_integrations(
        &self,
        grid: &mut Grid,
        page: Rect,
        (hover, measure): (Option<(u16, u16)>, &dyn Measure),
        theme: &Theme,
    ) {
        heading(
            grid,
            page,
            "Integrations",
            "Cellix works with your agents and terminal. Nothing changes until you click.",
            theme,
        );
        let blocks = self.blocks();
        for (index, block) in blocks.iter().enumerate() {
            let color = match block.tone {
                Tone::Good => theme.git_added,
                Tone::Quiet => theme.gutter.fg,
            };
            let text = (block.name, block.state, color);
            draw_block(
                grid,
                page,
                (index as u16, measure),
                text,
                block.detail,
                theme,
            );
            if let Some((label, primary, control)) = block.action {
                let spots = self.controls_at(page);
                let spot = spots.iter().find(|(_, c)| *c == control);
                if let Some((rect, _)) = spot {
                    draw_button(grid, *rect, label, primary, hover, theme);
                }
            }
        }
        if !self.note.is_empty() {
            let style = Style {
                fg: theme.accent,
                ..theme.text
            };
            let y = page.y + 3 + blocks.len() as u16 * BLOCK_STEP;
            grid.label(page.x, y, &self.note, style, page.right());
        }
    }

    /// Where the cards' buttons sit on a page already laid out.
    fn controls_at(&self, page: Rect) -> Vec<(Rect, Control)> {
        let x = page.right().saturating_sub(BUTTON_WIDTH + 2);
        let blocks = self.blocks();
        let placed = blocks.iter().enumerate();
        placed
            .filter_map(|(index, block)| {
                let (_, _, control) = block.action?;
                let y = page.y + 3 + index as u16 * BLOCK_STEP;
                Some((Rect::new(x, y, BUTTON_WIDTH, 2), control))
            })
            .collect()
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

/// A title with a line under it, at the left of a row of two, stopping before `room`.
fn draw_text(
    grid: &mut Grid,
    page: Rect,
    (y, room): (u16, u16),
    (title, detail): (&str, &str),
    theme: &Theme,
) {
    let name = Style {
        bold: true,
        ..theme.text
    };
    let quiet = Style {
        fg: theme.gutter.fg,
        ..theme.text
    };
    grid.label(page.x, y, title, name, room);
    grid.label(page.x, y + 1, detail, quiet, room);
}

fn draw_toggle(grid: &mut Grid, page: Rect, y: u16, text: (&str, &str), on: bool, theme: &Theme) {
    let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
    grid.hairline(above, theme.border);
    let room = page.right().saturating_sub(TOGGLE_WIDTH.ceil() as u16 + 2);
    draw_text(grid, page, (y, room), text, theme);
    let right = page.right() as f32;
    let x = right - TOGGLE_WIDTH;
    let track = if on { theme.button } else { theme.popup_border };
    let bounds = Bounds::cells(x, y as f32 + 0.12, TOGGLE_WIDTH, 0.76);
    grid.panel(bounds, 38, Some(track), None);
    let knob_x = if on { right - 1.2 } else { x + 0.17 };
    let knob_color = if on { theme.button_text } else { theme.text.fg };
    let knob = Bounds::cells(knob_x, y as f32 + 0.2, 1.03, 0.6);
    grid.panel(knob, 30, Some(knob_color), None);
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
    if on {
        // Twice over, so the choice shows at a glance.
        let inner = Bounds::cells(x + 0.06, y + 0.03, w - 0.12, rows - 0.06);
        grid.panel(inner, RADIUS, None, edge);
    }
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

fn badge(grid: &mut Grid, x: f32, y: u16, word: &str, color: Rgb, theme: &Theme) {
    let dot = Bounds::cells(x, y as f32 + 0.36, 0.42, 0.28);
    grid.panel(dot, 14, Some(color), None);
    let style = Style {
        fg: color,
        ..theme.text
    };
    let cell = x.ceil() as u16 + 1;
    grid.label_at(x + 1.0, cell, y, word, style, grid.width(), NOTE_SIZE);
}

/// One integration: a name, its state and a line about it, in a raised box.
fn draw_block(
    grid: &mut Grid,
    page: Rect,
    (index, measure): (u16, &dyn Measure),
    (name, word, color): (&str, &str, Rgb),
    detail: &str,
    theme: &Theme,
) {
    let top = page.y + 3 + index * BLOCK_STEP;
    let surface = Bounds::cells(page.x as f32, top as f32 - 0.4, page.w as f32, 2.8);
    grid.panel(surface, RADIUS, Some(theme.tab.bg), Some(theme.border));
    let style = Style {
        fg: theme.heading,
        bold: true,
        bg: theme.tab.bg,
        ..theme.text
    };
    let x = page.x + 2;
    let end = grid.label(x, top, name, style, page.right());
    let bold = Font {
        bold: true,
        ..Font::default()
    };
    let at = if measure.in_cells() {
        end as f32 + 1.0
    } else {
        x as f32 + measure.width(name, bold) + STATUS_GAP
    };
    badge(grid, at, top, word, color, theme);
    let quiet = Style {
        fg: theme.gutter.fg,
        bg: theme.tab.bg,
        ..theme.text
    };
    let room = page.right().saturating_sub(BUTTON_WIDTH + 3);
    grid.label(x, top + 1, detail, quiet, room);
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
        (false, false) => theme.quiet_button(theme.tab.bg),
    };
    let fg = if primary {
        theme.button_text
    } else {
        theme.text.fg
    };
    let style = Style { fg, ..theme.text };
    let cells = format!("{label:^width$}", width = rect.w as usize);
    grid.large_button_between(rect.x, rect.y, &cells, label, fill, style);
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
            found: Found::default(),
            ..SettingsPage::new(Found::default())
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
