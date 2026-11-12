use std::sync::mpsc::Receiver;

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::integrations;
use crate::keys::{self, Shortcut};
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
/// When files save themselves, as `auto_save` names it and as the page shows it.
const AUTO_SAVES: [(&str, &str); 3] = [
    ("off", "Off"),
    ("after_delay", "After a pause"),
    ("on_focus_change", "On leaving"),
];
/// Where the Editor section's rows start, from the top of the page.
const FORMAT_ROW: u16 = 3;
const AUTO_SAVE_ROW: u16 = 6;
const READING_ROW: u16 = 9;
/// The Keyboard section: the table's heading, and its first row.
const KEYS_HEAD_ROW: u16 = 6;
const KEYS_FIRST_ROW: u16 = 7;
/// Rows the filter's field takes over every page: its own, and one of air under it.
const FILTER_ROWS: u16 = 2;
/// Columns at the field's end that clear it.
const CLEAR_WIDTH: u16 = 3;
/// What the field says before anything is typed.
const FILTER_HINT: &str = "Search settings";
/// Where a language server's program starts on its row, from the page's left: on the Editor
/// page, and among what the filter found, where its page's name comes first.
const SERVER_DETAIL: (u16, u16) = (16, 24);
/// What each switch and choice is called and says, wherever it's drawn.
const THEME: (&str, &str) = ("Theme", "Follows the terminal, or stays dark or light.");
const SMOOTH: (&str, &str) = (
    "Smooth scrolling",
    "The wheel moves by pixels. Turn this off on a slow link.",
);
const MINIMAP: (&str, &str) = ("Minimap", "The file at a glance, when the window has room.");
const NOTIFY: (&str, &str) = (
    "Notifications",
    "When something needs you and Cellix isn't the window in front, the terminal says so.",
);
const FORMAT: (&str, &str) = (
    "Format on save",
    "Formats code with its language server or formatter as it's saved.",
);
const AUTO_SAVE: (&str, &str) = ("Auto save", "Saves on its own.");
const READING: (&str, &str) = (
    "Reading width",
    "Pages keep their text to 88 columns, in the middle of a wider pane.",
);
const KEYMAP: (&str, &str) = ("Keymap", "Standard works as in VS Code.");
/// The theme cards' names as a row of choices has room for them.
const THEME_SHORT: [(&str, &str); 3] = [("auto", "Auto"), ("dark", "Dark"), ("light", "Light")];
/// The Source column's width, and the reset button's at the end of a row.
const SOURCE_WIDTH: u16 = 9;
const RESET_WIDTH: u16 = 3;
/// Wide enough for "After a pause" with a cell either side.
const AUTO_SAVE_WIDTH: u16 = 14;
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
const SERVERS_ROW: u16 = 13;
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
    BracketPairs,
    WordWrap,
    StickyScroll,
    InlayHints,
    Notify,
    Keymap(&'static str),
    FormatOnSave,
    AutoSave(&'static str),
    ReadingWidth,
    /// Empties the filter.
    ClearFilter,
    /// A command's keys, by its place in `shortcuts`: a click records new ones.
    Shortcut(usize),
    /// Gives a command its own keys back.
    ResetShortcut(usize),
    ClaudeCode,
    Codex,
    Gemini,
    Ghostty,
    /// A language server to install, as an index into `SERVERS`.
    InstallServer(usize),
    /// terminal-browser, which runs a browser in the side pane.
    InstallBrowser,
    /// A command to put on the clipboard.
    CopyInstall(&'static str),
}

/// The settings as they stand, passed in so the page holds no second copy.
pub struct Values<'a> {
    pub theme: &'a str,
    pub keymap: &'a str,
    pub smooth_scroll: bool,
    pub minimap: bool,
    pub view: EditorView,
    pub inlay_hints: bool,
    pub format_on_save: bool,
    pub reading_width: bool,
    pub auto_save: &'a str,
    pub notify: bool,
}

/// How code is laid out and coloured, each a switch in Appearance.
#[derive(Clone, Copy, Debug, Default)]
pub struct EditorView {
    pub bracket_pairs: bool,
    pub word_wrap: bool,
    pub sticky_scroll: bool,
}

/// The switches under the theme cards, in order, with their words.
fn view_switches(values: &Values) -> [(Control, (&'static str, &'static str), bool); 4] {
    [
        (Control::WordWrap, WORD_WRAP, values.view.word_wrap),
        (
            Control::StickyScroll,
            STICKY_SCROLL,
            values.view.sticky_scroll,
        ),
        (
            Control::BracketPairs,
            BRACKET_PAIRS,
            values.view.bracket_pairs,
        ),
        (Control::InlayHints, INLAY_HINTS, values.inlay_hints),
    ]
}

/// How many switches `view_switches` holds.
const VIEW_SWITCHES: [Control; 4] = [
    Control::WordWrap,
    Control::StickyScroll,
    Control::BracketPairs,
    Control::InlayHints,
];

/// One row of what the filter found, whichever page it's from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Theme,
    /// A switch, by what it does.
    Switch(Control),
    AutoSave,
    Keymap,
    /// A language server, by its place in `SERVERS`.
    Server(usize),
    /// An integration's card, by its place among them.
    Block(usize),
    /// A command and its keys, by its place in `shortcuts`.
    Shortcut(usize),
}

impl Row {
    fn section(self) -> Section {
        match self {
            Row::Theme => Section::Appearance,
            Row::Switch(Control::FormatOnSave | Control::ReadingWidth) => Section::Editor,
            Row::Switch(_) => Section::Appearance,
            Row::AutoSave | Row::Server(_) => Section::Editor,
            Row::Block(_) => Section::Integrations,
            Row::Keymap | Row::Shortcut(_) => Section::Keyboard,
        }
    }

    /// Rows it takes, with the air under it.
    fn rows(self) -> u16 {
        match self {
            Row::Server(_) | Row::Shortcut(_) => 1,
            Row::Block(_) => BLOCK_STEP,
            _ => 3,
        }
    }
}

/// Every switch in Settings, in the order its pages have them, with its words.
const SWITCHES: [(Control, (&str, &str)); 9] = [
    (Control::SmoothScroll, SMOOTH),
    (Control::Minimap, MINIMAP),
    (Control::WordWrap, WORD_WRAP),
    (Control::StickyScroll, STICKY_SCROLL),
    (Control::BracketPairs, BRACKET_PAIRS),
    (Control::InlayHints, INLAY_HINTS),
    (Control::Notify, NOTIFY),
    (Control::FormatOnSave, FORMAT),
    (Control::ReadingWidth, READING),
];
const WORD_WRAP: (&str, &str) = (
    "Word wrap",
    "Long lines of code wrap at the pane's width. Alt+Z changes one file.",
);
const STICKY_SCROLL: (&str, &str) = (
    "Sticky scroll",
    "The lines that open the scopes you've scrolled into stay at the top.",
);
const BRACKET_PAIRS: (&str, &str) = (
    "Bracket pair colours",
    "Brackets in code coloured by depth, with the caret's pair marked.",
);
const INLAY_HINTS: (&str, &str) = (
    "Inlay hints",
    "Types and parameter names inside the lines, from the language server.",
);

/// Whether a switch is on, as the settings stand.
fn switch_on(control: Control, values: &Values) -> bool {
    match control {
        Control::SmoothScroll => values.smooth_scroll,
        Control::Minimap => values.minimap,
        Control::WordWrap => values.view.word_wrap,
        Control::StickyScroll => values.view.sticky_scroll,
        Control::BracketPairs => values.view.bracket_pairs,
        Control::InlayHints => values.inlay_hints,
        Control::Notify => values.notify,
        Control::FormatOnSave => values.format_on_save,
        Control::ReadingWidth => values.reading_width,
        _ => false,
    }
}

/// What a section is called.
fn section_name(section: Section) -> &'static str {
    SECTIONS.iter().find(|s| s.1 == section).map_or("", |s| s.0)
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
    /// Whether terminal-browser is on the path.
    pub browser: bool,
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
            browser: integrations::on_path("terminal-browser", var),
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
    /// Every command and its keys, as the Keyboard section lists them.
    pub shortcuts: Vec<Shortcut>,
    /// What's typed in the filter, which narrows the rows of every page to those that fit.
    pub filter: String,
    /// The first of what it found that's in view.
    pub found_top: usize,
    /// The first command in view in the Keyboard section.
    pub keys_top: usize,
    /// A command whose new keys are being pressed, and those pressed so far.
    pub recording: Option<(usize, Option<String>)>,
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
            shortcuts: Vec::new(),
            filter: String::new(),
            found_top: 0,
            keys_top: 0,
            recording: None,
        }
    }

    /// The commands that fit the filter, by their place in `shortcuts`: by name, or by keys.
    pub fn shown_shortcuts(&self) -> Vec<usize> {
        let query = &self.filter;
        let fits = |(_, s): &(usize, &Shortcut)| keys::matches(s, query);
        self.shortcuts
            .iter()
            .enumerate()
            .filter(fits)
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether something is typed in the filter, so the rows it found stand in the page's place.
    pub fn filtering(&self) -> bool {
        !self.filter.trim().is_empty()
    }

    /// Every row of every page that fits the filter, in the pages' order: a switch or choice
    /// by its name or what it says, a language server by its language or program, an
    /// integration by its name and state, a command by its name or keys.
    fn found_rows(&self) -> Vec<Row> {
        let query = self.filter.trim().to_lowercase();
        let fits = |texts: &[&str]| texts.iter().any(|t| t.to_lowercase().contains(&query));
        let pair = |(name, says): (&str, &str)| fits(&[name, says]);
        let mut rows = Vec::new();
        if pair(THEME) || fits(&["dark", "light"]) {
            rows.push(Row::Theme);
        }
        let switches = |section: Section, rows: &mut Vec<Row>| {
            for (control, words) in SWITCHES {
                if Row::Switch(control).section() == section && pair(words) {
                    rows.push(Row::Switch(control));
                }
            }
        };
        switches(Section::Appearance, &mut rows);
        // The Editor page has Auto save between its two switches.
        if pair(FORMAT) {
            rows.push(Row::Switch(Control::FormatOnSave));
        }
        if pair(AUTO_SAVE) || AUTO_SAVES.iter().any(|a| fits(&[a.1])) {
            rows.push(Row::AutoSave);
        }
        if pair(READING) {
            rows.push(Row::Switch(Control::ReadingWidth));
        }
        for (index, server) in SERVERS.iter().enumerate() {
            if fits(server.languages) || fits(&[server.command[0]]) {
                rows.push(Row::Server(index));
            }
        }
        for (index, block) in self.blocks().iter().enumerate() {
            if fits(&[block.name, block.state, block.detail]) {
                rows.push(Row::Block(index));
            }
        }
        if pair(KEYMAP) || KEYMAPS.iter().any(|k| fits(&[k.1])) {
            rows.push(Row::Keymap);
        }
        rows.extend(self.shown_shortcuts().into_iter().map(Row::Shortcut));
        rows
    }

    /// How many rows of each page the filter found.
    fn found_in(&self, section: Section) -> usize {
        let rows = self.found_rows();
        rows.iter().filter(|r| r.section() == section).count()
    }

    /// The first of what the filter found on a page, as a place to scroll to.
    pub fn first_found_in(&self, section: Section) -> Option<usize> {
        self.found_rows()
            .iter()
            .position(|r| r.section() == section)
    }

    /// What the filter found that's in view, each with its row. A run of one-row rows keeps
    /// a row of air from a taller one after it.
    fn found_layout(&self, page: Rect) -> Vec<(Row, u16)> {
        let mut placed = Vec::new();
        let mut y = page.y;
        let mut short = false;
        for row in self.found_rows().into_iter().skip(self.found_top) {
            if short && row.rows() > 1 {
                y += 1;
            }
            if y + row.rows().min(2) > page.bottom() {
                break;
            }
            placed.push((row, y));
            short = row.rows() == 1;
            y += row.rows();
        }
        placed
    }

    /// The filter's field, over every page.
    fn filter_field(area: Rect) -> Rect {
        let page = Self::content(area);
        Rect::new(page.x, page.y - FILTER_ROWS, page.w, 1)
    }

    /// The shortcut rows in view: each one's row, and its place in `shortcuts`.
    fn shortcut_rows(&self, page: Rect) -> Vec<(u16, usize)> {
        let room = page.h.saturating_sub(KEYS_FIRST_ROW) as usize;
        let shown = self
            .shown_shortcuts()
            .into_iter()
            .skip(self.keys_top)
            .take(room);
        shown
            .enumerate()
            .map(|(row, index)| (page.y + KEYS_FIRST_ROW + row as u16, index))
            .collect()
    }

    /// The table's columns: where the keys start, and where the source does.
    fn key_columns(page: Rect) -> (u16, u16) {
        let source = page.right().saturating_sub(SOURCE_WIDTH + RESET_WIDTH);
        let keys = page.x + (page.w * 2 / 5).clamp(16, 34);
        (keys, source)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        if self.filtering() {
            let last = self.found_rows().len().saturating_sub(1) as isize;
            self.found_top = (self.found_top as isize + delta).clamp(0, last) as usize;
            return;
        }
        if self.section == Section::Keyboard {
            let last = self.shown_shortcuts().len().saturating_sub(1) as isize;
            self.keys_top = (self.keys_top as isize + delta).clamp(0, last) as usize;
            return;
        }
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
        blocks.push(if found.browser {
            Block {
                name: "Browser",
                state: "terminal-browser installed",
                tone: Tone::Good,
                detail: "Pages and previews open in the side pane.",
                action: None,
            }
        } else {
            Block {
                name: "Browser",
                state: "terminal-browser not found",
                tone: Tone::Quiet,
                detail: "Runs a real browser in the side pane, in any terminal.",
                action: Some(("Install", true, Control::InstallBrowser)),
            }
        });
        match found.ghostty_keeps_cmd_d {
            Some(true) => blocks.push(Block {
                name: "Ghostty keys",
                state: "Cmd+A and Cmd+D go to Ghostty",
                tone: Tone::Quiet,
                detail: "Ghostty keeps Cmd+A and Cmd+D for itself. Ctrl+A and Ctrl+D always work.",
                action: Some(("Free Them", false, Control::Ghostty)),
            }),
            Some(false) => blocks.push(Block {
                name: "Ghostty keys",
                state: "Cmd+A and Cmd+D are free",
                tone: Tone::Good,
                detail: "Reload Ghostty's config if they don't select yet.",
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

    /// Where the page's content goes, under the tabs and the filter's field.
    fn content(area: Rect) -> Rect {
        let side = if area.w >= SIDE_NAV { NAV_WIDTH + 2 } else { 0 };
        let top = area.y + FILTER_ROWS + if side > 0 { 2 } else { 4 };
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
        if self.filtering() {
            let field = Self::filter_field(area);
            let clear = Rect::new(
                field.right().saturating_sub(CLEAR_WIDTH),
                field.y,
                CLEAR_WIDTH,
                1,
            );
            spots.push((clear, Control::ClearFilter));
            self.found_controls(page, &mut spots);
            return spots;
        }
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
                for (i, control) in VIEW_SWITCHES.iter().enumerate() {
                    let y = below + 6 + 3 * i as u16;
                    spots.push((Rect::new(page.x, y, page.w, 2), *control));
                }
                let notify = below + 6 + 3 * VIEW_SWITCHES.len() as u16;
                spots.push((Rect::new(page.x, notify, page.w, 2), Control::Notify));
            }
            Section::Editor => {
                let format = Rect::new(page.x, page.y + FORMAT_ROW, page.w, 2);
                spots.push((format, Control::FormatOnSave));
                let start = page.right().saturating_sub(AUTO_SAVE_WIDTH * 3);
                for (i, (name, _)) in AUTO_SAVES.iter().enumerate() {
                    let x = start + i as u16 * AUTO_SAVE_WIDTH;
                    let rect = Rect::new(x, page.y + AUTO_SAVE_ROW, AUTO_SAVE_WIDTH, 1);
                    spots.push((rect, Control::AutoSave(name)));
                }
                let reading = Rect::new(page.x, page.y + READING_ROW, page.w, 2);
                spots.push((reading, Control::ReadingWidth));
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
            Section::Keyboard => {
                let start = page.right().saturating_sub(SEGMENT_WIDTH * 3);
                for (i, (name, _)) in KEYMAPS.iter().enumerate() {
                    let x = start + i as u16 * SEGMENT_WIDTH;
                    let rect = Rect::new(x, page.y + 3, SEGMENT_WIDTH, 1);
                    spots.push((rect, Control::Keymap(name)));
                }
                let (keys_x, source_x) = Self::key_columns(page);
                for (y, index) in self.shortcut_rows(page) {
                    let width = source_x.saturating_sub(keys_x + 1);
                    spots.push((Rect::new(keys_x, y, width, 1), Control::Shortcut(index)));
                    if self.shortcuts[index].user {
                        let x = page.right().saturating_sub(RESET_WIDTH);
                        let rect = Rect::new(x, y, RESET_WIDTH, 1);
                        spots.push((rect, Control::ResetShortcut(index)));
                    }
                }
            }
        }
        spots
    }

    /// The controls of what the filter found, each on the row it's laid out at.
    fn found_controls(&self, page: Rect, spots: &mut Vec<(Rect, Control)>) {
        let segments = |y: u16, width: u16, spots: &mut Vec<(Rect, Control)>, of: [Control; 3]| {
            let start = page.right().saturating_sub(width * 3);
            for (i, control) in of.into_iter().enumerate() {
                let rect = Rect::new(start + i as u16 * width, y, width, 1);
                spots.push((rect, control));
            }
        };
        let blocks = self.blocks();
        for (row, y) in self.found_layout(page) {
            match row {
                Row::Theme => {
                    let of = THEMES.map(|t| Control::Theme(t.0));
                    segments(y, SEGMENT_WIDTH, spots, of);
                }
                Row::Switch(control) => spots.push((Rect::new(page.x, y, page.w, 2), control)),
                Row::AutoSave => {
                    let of = AUTO_SAVES.map(|a| Control::AutoSave(a.0));
                    segments(y, AUTO_SAVE_WIDTH, spots, of);
                }
                Row::Keymap => {
                    let of = KEYMAPS.map(|k| Control::Keymap(k.0));
                    segments(y, SEGMENT_WIDTH, spots, of);
                }
                Row::Server(index) => {
                    if !self.found.servers.get(index).copied().unwrap_or(false) {
                        let x = page.right().saturating_sub(INSTALL_WIDTH);
                        let rect = Rect::new(x, y, INSTALL_WIDTH, 1);
                        spots.push((rect, Control::InstallServer(index)));
                    }
                }
                Row::Block(index) => {
                    if let Some((_, _, control)) = blocks[index].action {
                        let x = page.right().saturating_sub(BUTTON_WIDTH + 2);
                        spots.push((Rect::new(x, y, BUTTON_WIDTH, 2), control));
                    }
                }
                Row::Shortcut(index) => {
                    let (keys_x, source_x) = Self::key_columns(page);
                    let width = source_x.saturating_sub(keys_x + 1);
                    spots.push((Rect::new(keys_x, y, width, 1), Control::Shortcut(index)));
                    if self.shortcuts[index].user {
                        let x = page.right().saturating_sub(RESET_WIDTH);
                        let rect = Rect::new(x, y, RESET_WIDTH, 1);
                        spots.push((rect, Control::ResetShortcut(index)));
                    }
                }
            }
        }
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
        self.draw_filter(grid, area, (hover, measure), theme);
        if self.filtering() {
            let state = (&spots[..], values, hover, measure);
            self.draw_found(grid, page, state, theme);
            self.draw_focus(grid, &spots, theme);
            return;
        }
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
                draw_toggle(grid, page, below, SMOOTH, values.smooth_scroll, theme);
                draw_toggle(grid, page, below + 3, MINIMAP, values.minimap, theme);
                for (i, (_, text, on)) in view_switches(values).into_iter().enumerate() {
                    draw_toggle(grid, page, below + 6 + 3 * i as u16, text, on, theme);
                }
                let y = below + 6 + 3 * VIEW_SWITCHES.len() as u16;
                draw_toggle(grid, page, y, NOTIFY, values.notify, theme);
            }
            Section::Editor => {
                self.draw_editor(grid, page, &spots, values, (hover, measure), theme)
            }
            Section::Integrations => self.draw_integrations(grid, page, (hover, measure), theme),
            Section::Keyboard => {
                let state = (&spots[..], values, hover, measure);
                self.draw_keyboard(grid, page, state, theme);
            }
        }
        self.draw_focus(grid, &spots, theme);
    }

    /// The ring round the control the keyboard is on.
    fn draw_focus(&self, grid: &mut Grid, spots: &[(Rect, Control)], theme: &Theme) {
        // A row taking new keys has its own outline: a second ring would only blur it.
        let ringed = self.focus.and_then(|f| spots.get(f));
        let ringed = ringed.filter(|_| self.recording.is_none());
        if let Some((rect, _)) = ringed {
            let ring = Bounds::inset(*rect, -0.15, -0.1);
            grid.panel(ring, RADIUS, None, Some(theme.focus));
        }
    }

    /// The filter's field over every page: what's typed, how many rows fit it, and an ×
    /// that empties it. Typing anywhere in Settings lands here, so the caret is always in it.
    fn draw_filter(
        &self,
        grid: &mut Grid,
        area: Rect,
        (hover, measure): (Option<(u16, u16)>, &dyn Measure),
        theme: &Theme,
    ) {
        let field = Self::filter_field(area);
        let y = field.y;
        let bounds = Bounds::cells(field.x as f32, y as f32 + 0.06, field.w as f32, 0.88);
        grid.panel(bounds, 18, Some(theme.input), Some(theme.focus));
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let icon = field.x as f32 + 0.6;
        grid.glyph(icon, y, Icon::Search, theme.gutter.fg, "⌕", theme.text);
        let (text, cell) = (icon + 2.3, field.x + 4);
        let mut end = field.right().saturating_sub(1);
        if self.filtering() {
            let count = match self.found_rows().len() {
                0 => "No settings".to_string(),
                1 => "1 setting".to_string(),
                n => format!("{n} settings"),
            };
            let clear = Rect::new(field.right().saturating_sub(CLEAR_WIDTH), y, CLEAR_WIDTH, 1);
            let on = hover.is_some_and(|h| clear.contains(h.0, h.1));
            let colour = if on { theme.text.fg } else { theme.gutter.fg };
            grid.glyph(
                clear.x as f32 + 0.5,
                y,
                Icon::Close,
                colour,
                "×",
                theme.text,
            );
            let right = clear.x.saturating_sub(1);
            grid.label_right(right, y, &count, quiet, NOTE_SIZE);
            end = right.saturating_sub(count.chars().count() as u16 + 1);
        }
        if self.filter.is_empty() {
            grid.label_at(text + 0.4, cell, y, FILTER_HINT, quiet, end, 100);
        } else {
            grid.label_at(text, cell, y, &self.filter, theme.text, end, 100);
        }
        if self.recording.is_none() {
            // Where the text really ends, which proportional letters make short of its cells.
            let caret = text + measure.width(&self.filter, Font::default());
            grid.set_caret(caret, y, 0.56);
        }
    }

    /// What the filter found, in the page's place: each row as its own page draws it, with
    /// that page's name beside its own.
    #[allow(clippy::type_complexity)]
    fn draw_found(
        &self,
        grid: &mut Grid,
        page: Rect,
        (spots, values, hover, measure): (
            &[(Rect, Control)],
            &Values,
            Option<(u16, u16)>,
            &dyn Measure,
        ),
        theme: &Theme,
    ) {
        let placed = self.found_layout(page);
        if placed.is_empty() {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.text
            };
            let none = "Nothing in Settings fits that. Esc clears it.";
            grid.label(page.x, page.y, none, quiet, page.right());
            return;
        }
        let bold = Font {
            bold: true,
            ..Font::default()
        };
        // The page a row is from, after its name, when there's room before what follows.
        let tag = |grid: &mut Grid, (name, font): (&str, Font), (y, stop): (u16, u16), row: Row| {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.text
            };
            let word = section_name(row.section());
            let x = page.x as f32 + measure.width(name, font) + 1.0;
            let size = Font {
                size: NOTE_SIZE,
                ..Font::default()
            };
            if x + measure.width(word, size) + 1.0 <= f32::from(stop) {
                grid.label_at(x, x.ceil() as u16, y, word, quiet, stop, NOTE_SIZE);
            }
        };
        let rule = |grid: &mut Grid, y: u16| {
            let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
            grid.hairline(above, theme.border);
        };
        let segments =
            |grid: &mut Grid, y: u16, label: &dyn Fn(Control) -> Option<(&'static str, bool)>| {
                for (rect, control) in spots.iter().filter(|(r, _)| r.y == y && r.h == 1) {
                    if let Some((word, on)) = label(*control) {
                        draw_segment(grid, *rect, word, on, theme);
                    }
                }
            };
        let shared = keys::sharing(&self.shortcuts);
        let blocks = self.blocks();
        for (row, y) in placed {
            match row {
                Row::Theme => {
                    rule(grid, y);
                    let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
                    draw_text(grid, page, (y, room), THEME, theme);
                    tag(grid, (THEME.0, bold), (y, room), row);
                    segments(grid, y, &|control| match control {
                        Control::Theme(name) => {
                            let word = THEME_SHORT.iter().find(|t| t.0 == name)?.1;
                            Some((word, values.theme == name))
                        }
                        _ => None,
                    });
                }
                Row::Switch(control) => {
                    let words = SWITCHES.iter().find(|s| s.0 == control).map(|s| s.1);
                    let words = words.unwrap_or_default();
                    draw_toggle(grid, page, y, words, switch_on(control, values), theme);
                    let stop = page.right().saturating_sub(TOGGLE_WIDTH.ceil() as u16 + 2);
                    tag(grid, (words.0, bold), (y, stop), row);
                }
                Row::AutoSave => {
                    rule(grid, y);
                    let room = page.right().saturating_sub(AUTO_SAVE_WIDTH * 3 + 1);
                    draw_text(grid, page, (y, room), AUTO_SAVE, theme);
                    tag(grid, (AUTO_SAVE.0, bold), (y, room), row);
                    segments(grid, y, &|control| match control {
                        Control::AutoSave(name) => {
                            let word = AUTO_SAVES.iter().find(|a| a.0 == name)?.1;
                            Some((word, values.auto_save == name))
                        }
                        _ => None,
                    });
                }
                Row::Keymap => {
                    rule(grid, y);
                    let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
                    draw_text(grid, page, (y, room), KEYMAP, theme);
                    tag(grid, (KEYMAP.0, bold), (y, room), row);
                    segments(grid, y, &|control| match control {
                        Control::Keymap(name) => {
                            let word = KEYMAPS.iter().find(|k| k.0 == name)?.1;
                            Some((word, values.keymap == name))
                        }
                        _ => None,
                    });
                }
                Row::Server(index) => {
                    let place = (y, index, SERVER_DETAIL.1);
                    self.draw_server(grid, page, place, spots, (hover, measure), theme);
                    let name = SERVERS[index].languages[0];
                    tag(
                        grid,
                        (name, Font::default()),
                        (y, page.x + SERVER_DETAIL.1 - 1),
                        row,
                    );
                }
                Row::Block(index) => {
                    let block = &blocks[index];
                    let colour = match block.tone {
                        Tone::Good => theme.git_added,
                        Tone::Quiet => theme.gutter.fg,
                    };
                    let text = (block.name, block.state, colour);
                    draw_block(grid, page, (y, measure), text, block.detail, theme);
                    if let Some((label, _, control)) = block.action
                        && let Some((rect, _)) =
                            spots.iter().find(|(r, c)| *c == control && r.y == y)
                    {
                        draw_button(grid, *rect, label, false, (hover, measure), theme);
                    }
                }
                Row::Shortcut(index) => {
                    self.draw_shortcut(grid, page, (y, index), (hover, measure, &shared), theme);
                    let (keys_x, _) = Self::key_columns(page);
                    let name = self.shortcuts[index].name;
                    tag(
                        grid,
                        (name, Font::default()),
                        (y, keys_x.saturating_sub(2)),
                        row,
                    );
                }
            }
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
        // With a filter typed no page is the one showing: each says how many of its rows fit.
        let filtering = self.filtering();
        let found = if filtering { self.found_in(section) } else { 0 };
        let on = self.section == section && !filtering;
        let name = section_name(section);
        let wash = if on {
            Some(theme.tint)
        } else {
            hovered.then_some(theme.page_hover)
        };
        if let Some(fill) = wash {
            grid.panel(Bounds::inset(rect, 0.0, 0.055), 18, Some(fill), None);
        }
        let fg = match (on, filtering && found == 0) {
            (true, _) => theme.accent,
            (false, true) => theme.gutter.fg,
            (false, false) => theme.text.fg,
        };
        let style = Style { fg, ..theme.text };
        let x = rect.x as f32 + 1.0;
        grid.label_at(x, rect.x + 1, rect.y, name, style, rect.right(), 100);
        // Beside the name where the sections run down the side; across the top there's no room.
        if found > 0 && rect.w >= NAV_WIDTH {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.text
            };
            let count = found.to_string();
            grid.label_right(
                rect.right().saturating_sub(1),
                rect.y,
                &count,
                quiet,
                NOTE_SIZE,
            );
        }
    }

    /// Every command and its keys, searchable, with a click to set new ones.
    #[allow(clippy::type_complexity)]
    fn draw_keyboard(
        &self,
        grid: &mut Grid,
        page: Rect,
        (spots, values, hover, measure): (
            &[(Rect, Control)],
            &Values,
            Option<(u16, u16)>,
            &dyn Measure,
        ),
        theme: &Theme,
    ) {
        heading(
            grid,
            page,
            "Keyboard",
            "Click a command's keys to change them. Cmd works as Ctrl where the terminal passes it.",
            theme,
        );
        let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
        draw_text(grid, page, (page.y + 3, room), KEYMAP, theme);
        for (rect, control) in spots {
            if let Control::Keymap(name) = control {
                let label = KEYMAPS.iter().find(|k| k.0 == *name).map_or("", |k| k.1);
                draw_segment(grid, *rect, label, values.keymap == *name, theme);
            }
        }
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let (keys_x, source_x) = Self::key_columns(page);
        let head = Style {
            bold: true,
            ..quiet
        };
        let y = page.y + KEYS_HEAD_ROW;
        grid.label_at(page.x as f32, page.x, y, "COMMAND", head, keys_x, 80);
        grid.label_at(keys_x as f32, keys_x, y, "KEYS", head, source_x, 80);
        grid.label_at(
            source_x as f32,
            source_x,
            y,
            "SOURCE",
            head,
            page.right(),
            80,
        );
        let under = Bounds::cells(page.x as f32, y as f32 + 1.0, page.w as f32, 0.0);
        grid.hairline(under, theme.border);
        let shared = keys::sharing(&self.shortcuts);
        for (y, index) in self.shortcut_rows(page) {
            self.draw_shortcut(grid, page, (y, index), (hover, measure, &shared), theme);
        }
    }

    /// One command's row: its name, its keys or the keys being pressed for it, and whether
    /// they're its own or the person's, with a way back to its own.
    fn draw_shortcut(
        &self,
        grid: &mut Grid,
        page: Rect,
        (y, index): (u16, usize),
        (hover, measure, shared): Shown<'_>,
        theme: &Theme,
    ) {
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let (keys_x, source_x) = Self::key_columns(page);
        let shortcut = &self.shortcuts[index];
        let row = Rect::new(page.x, y, page.w, 1);
        if hover.is_some_and(|h| row.contains(h.0, h.1)) {
            let wash = Bounds::cells(
                page.x as f32 - 0.6,
                y as f32 + 0.055,
                page.w as f32 + 1.2,
                0.89,
            );
            grid.panel(wash, 18, Some(theme.page_hover), None);
        }
        grid.label(
            page.x,
            y,
            shortcut.name,
            theme.text,
            keys_x.saturating_sub(1),
        );
        let keys_end = source_x.saturating_sub(1);
        match &self.recording {
            Some((recording, pressed)) if *recording == index => {
                // One outline, inside the row, the keys on their column with air before.
                let box_ = Bounds::cells(
                    keys_x as f32 - 1.0,
                    y as f32 + 0.1,
                    (keys_end - keys_x) as f32 + 1.0,
                    0.8,
                );
                grid.panel(box_, 18, Some(theme.input), Some(theme.focus));
                match pressed {
                    Some(keys) => grid.label(keys_x, y, keys, theme.text, keys_end),
                    None => grid.label(keys_x, y, "Press the keys", quiet, keys_end),
                };
                let hint = if pressed.is_some() {
                    "Enter saves · Esc cancels"
                } else {
                    "Esc cancels"
                };
                grid.label_right(keys_end.saturating_sub(1), y, hint, quiet, NOTE_SIZE);
            }
            _ => {
                let mut x = keys_x;
                if shortcut.keys.is_empty() {
                    grid.label(x, y, "—", quiet, keys_end);
                }
                for key in &shortcut.keys {
                    let width = measure.width(key, Font::default());
                    let cap = Bounds::cells(x as f32 - 0.6, y as f32 + 0.12, width + 1.2, 0.76);
                    grid.panel(cap, 24, Some(theme.tab.bg), Some(theme.popup_border));
                    x = grid.label(x, y, key, theme.text, keys_end) + 2;
                    if x >= keys_end {
                        break;
                    }
                }
                let clash = shortcut.keys.iter().filter_map(|k| shared.get(k)).max();
                if let Some(&many) = clash.filter(|n| **n > 1) {
                    let warn = Style {
                        fg: theme.attention,
                        ..theme.text
                    };
                    grid.label(x, y, &format!("⚠ {many} commands"), warn, keys_end);
                }
            }
        }
        let (source, tone) = if shortcut.user {
            ("User", theme.accent)
        } else {
            ("Default", theme.gutter.fg)
        };
        let style = Style {
            fg: tone,
            ..theme.text
        };
        grid.label(source_x, y, source, style, page.right());
        if shortcut.user {
            let x = page.right().saturating_sub(RESET_WIDTH);
            let reset = Rect::new(x, y, RESET_WIDTH, 1);
            let on = hover.is_some_and(|h| reset.contains(h.0, h.1));
            let style = Style {
                fg: if on { theme.text.fg } else { theme.gutter.fg },
                ..theme.text
            };
            grid.label(x + 1, y, "↺", style, page.right());
        }
    }

    fn draw_editor(
        &self,
        grid: &mut Grid,
        page: Rect,
        spots: &[(Rect, Control)],
        values: &Values,
        (hover, measure): (Option<(u16, u16)>, &dyn Measure),
        theme: &Theme,
    ) {
        heading(
            grid,
            page,
            "Editor",
            "How files save, how pages are set, and the languages.",
            theme,
        );
        draw_toggle(
            grid,
            page,
            page.y + FORMAT_ROW,
            FORMAT,
            values.format_on_save,
            theme,
        );
        let y = page.y + AUTO_SAVE_ROW;
        let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
        grid.hairline(above, theme.border);
        let room = page.right().saturating_sub(AUTO_SAVE_WIDTH * 3 + 1);
        draw_text(grid, page, (y, room), AUTO_SAVE, theme);
        for (rect, control) in spots {
            if let Control::AutoSave(name) = control {
                let label = AUTO_SAVES.iter().find(|a| a.0 == *name).map_or("", |a| a.1);
                draw_segment(grid, *rect, label, values.auto_save == *name, theme);
            }
        }
        let y = page.y + READING_ROW;
        let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
        grid.hairline(above, theme.border);
        draw_toggle(grid, page, y, READING, values.reading_width, theme);
        let line_y = page.y + SERVERS_ROW - 1;
        let above = Bounds::cells(page.x as f32, line_y as f32 - 0.5, page.w as f32, 0.0);
        grid.hairline(above, theme.border);
        let count = self.found.servers.iter().filter(|found| **found).count();
        // A section's heading as every panel has it, small, bold and quiet, with what it
        // counts at the row's end in plain words.
        let heading = Style {
            fg: theme.gutter.fg,
            bold: true,
            ..theme.text
        };
        let x = page.x as f32;
        let size = NOTE_SIZE - 10;
        grid.label_at(
            x,
            page.x,
            line_y,
            "LANGUAGE SERVERS",
            heading,
            page.right(),
            size,
        );
        let found = format!("{count} of {} found", SERVERS.len());
        let plain = Style {
            bold: false,
            ..heading
        };
        grid.label_right(page.right(), line_y, &found, plain, NOTE_SIZE);
        for (row, index) in self.server_rows(page).into_iter().enumerate() {
            let y = page.y + SERVERS_ROW + row as u16;
            let place = (y, index, SERVER_DETAIL.0);
            self.draw_server(grid, page, place, spots, (hover, measure), theme);
        }
    }

    /// One server: its languages and program, the program starting `detail` columns in, and
    /// whether it's here or a click from it.
    fn draw_server(
        &self,
        grid: &mut Grid,
        page: Rect,
        (y, index, detail): (u16, usize, u16),
        spots: &[(Rect, Control)],
        (hover, measure): (Option<(u16, u16)>, &dyn Measure),
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
        grid.label(page.x, y, server.languages[0], theme.text, page.x + detail);
        let found = self.found.servers.get(index).copied().unwrap_or(false);
        // Hovering a missing server shows what its button will run.
        let end = page.right().saturating_sub(INSTALL_WIDTH + 1);
        if hovered && !found {
            let runs = format!("Runs {}", server.install);
            grid.label(page.x + detail, y, &runs, theme.text, end);
        } else {
            grid.label(page.x + detail, y, server.command[0], quiet, end);
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
            // Words in the accent until its row is pointed at: twenty chips in a column
            // stood a few pixels apart, a wall beside the list.
            let (x, wide) = button_place(*rect, "Install", measure);
            let cells = format!("{:^width$}", "Install", width = wide as usize);
            if hovered {
                let style = Style {
                    fg: theme.button_text,
                    ..theme.text
                };
                grid.button(x, rect.y, &cells, "Install", theme.button, style);
            } else {
                // Its word where Found stands on the rows beside it.
                let word = Style {
                    fg: theme.accent,
                    ..theme.text
                };
                grid.label_right(page.right(), y, "Install", word, NOTE_SIZE);
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
        // One primary button to a page: the first that wants it; the rest are quiet.
        let mut primary_left = true;
        for (index, block) in blocks.iter().enumerate() {
            let color = match block.tone {
                Tone::Good => theme.git_added,
                Tone::Quiet => theme.gutter.fg,
            };
            let text = (block.name, block.state, color);
            let top = page.y + 3 + index as u16 * BLOCK_STEP;
            draw_block(grid, page, (top, measure), text, block.detail, theme);
            if let Some((label, primary, control)) = block.action {
                let spots = self.controls_at(page);
                let spot = spots.iter().find(|(_, c)| *c == control);
                if let Some((rect, _)) = spot {
                    let primary = primary && std::mem::take(&mut primary_left);
                    draw_button(grid, *rect, label, primary, (hover, measure), theme);
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

/// What a command's row is drawn with: the pointer, the text's measure, and how many
/// commands each set of keys is bound to, which marks keys two commands share.
type Shown<'a> = (
    Option<(u16, u16)>,
    &'a dyn Measure,
    &'a std::collections::HashMap<String, usize>,
);

/// One choice of a row of them, filled when it's the one chosen.
fn draw_segment(grid: &mut Grid, rect: Rect, label: &str, on: bool, theme: &Theme) {
    let fill = if on { theme.button } else { theme.tab.bg };
    let style = Style {
        fg: if on { theme.button_text } else { theme.text.fg },
        ..theme.text
    };
    let cells = format!("{label:^width$}", width = rect.w as usize);
    grid.large_button(rect.x, rect.y, &cells, label, fill, style);
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
    // The theme showing now, as it sits on this terminal, when the card is its kind: picking
    // it gives just that.
    let sample = match name {
        "dark" if !theme.is_light() => *theme,
        "light" if theme.is_light() => *theme,
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
    (top, measure): (u16, &dyn Measure),
    (name, word, color): (&str, &str, Rgb),
    detail: &str,
    theme: &Theme,
) {
    let surface = Bounds::cells(page.x as f32, top as f32 - 0.4, page.w as f32, 2.8);
    grid.panel(surface, RADIUS, Some(theme.popup), Some(theme.border));
    let style = Style {
        fg: theme.heading,
        bold: true,
        bg: theme.popup,
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
        bg: theme.popup,
        ..theme.text
    };
    // Two columns short of the button, and cut with an ellipsis, not mid-word at its edge.
    let room = page.right().saturating_sub(BUTTON_WIDTH + 3);
    let wide = f32::from(room.saturating_sub(x + 2));
    let detail = crate::context::elide(detail, wide, measure);
    grid.label(x, top + 1, &detail, quiet, room);
}

/// As many columns of `rect` as a button with these words takes, from its right end: the
/// place it's clicked stays `rect`, and the chip is no wider than its words need.
fn button_place(rect: Rect, label: &str, measure: &dyn Measure) -> (u16, u16) {
    if measure.in_cells() {
        return (rect.x, rect.w);
    }
    let wide = crate::measures::button_width(label, measure).min(rect.w);
    (rect.right() - wide, wide)
}

fn draw_button(
    grid: &mut Grid,
    rect: Rect,
    label: &str,
    primary: bool,
    (hover, measure): (Option<(u16, u16)>, &dyn Measure),
    theme: &Theme,
) {
    let hovered = hover.is_some_and(|h| rect.contains(h.0, h.1));
    let fill = match (primary, hovered) {
        (true, _) => theme.button,
        (false, true) => theme.pick,
        (false, false) => theme.quiet_button(theme.popup),
    };
    let fg = if primary {
        theme.button_text
    } else {
        theme.text.fg
    };
    let style = Style { fg, ..theme.text };
    let (x, wide) = button_place(rect, label, measure);
    let cells = format!("{label:^width$}", width = wide as usize);
    grid.large_button_between(x, rect.y, &cells, label, fill, style);
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
    fn the_browser_can_be_installed_from_integrations() {
        let mut settings = page(Section::Integrations);
        let offered = |s: &SettingsPage| {
            s.controls(AREA)
                .iter()
                .any(|c| c.1 == Control::InstallBrowser)
        };
        settings.found.browser = false;
        assert!(offered(&settings));
        settings.found.browser = true;
        assert!(!offered(&settings), "not once it's there");
    }

    #[test]
    fn every_listed_key_does_what_its_row_says() {
        let bindings = keys::Bindings::default();
        for shortcut in keys::shortcuts(&Default::default()) {
            for key in &shortcut.keys {
                let event = keys::parse(key).unwrap();
                assert_eq!(bindings.command(event), Some(shortcut.command), "{key}");
            }
        }
    }
}
