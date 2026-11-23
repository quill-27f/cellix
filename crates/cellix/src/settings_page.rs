use std::sync::mpsc::Receiver;

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::field::TextBox;
use crate::install::{Machine, Plan};
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
const CURSOR_ROW: u16 = 12;
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
const MOTION: (&str, &str) = (
    "Motion",
    "Spinners turn while something is under way. Off, each is a still dot.",
);
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
/// VS Code's Cursor Blinking, on or off: its blink, half a second each way.
const CURSOR_BLINK: (&str, &str) = (
    "Cursor blinking",
    "The cursor blinks in files, pages, text boxes and terminals. Off, it stays still.",
);
const KEYMAP: (&str, &str) = ("Keymap", "Standard works as in VS Code.");
const ROW_HEIGHT: (&str, &str) = ("Row height", "How much air each row has.");
/// The row heights as `row_height` names them and as the page shows them.
const ROW_HEIGHTS: [(&str, &str); 3] = [
    ("airy", "Airy"),
    ("comfortable", "Comfortable"),
    ("terminal", "Terminal's"),
];
/// Appearance's parts, in rows from the top of the page: the theme's cards under its
/// heading, Row height two rows under those, and a switch every three rows after it.
const CARDS_ROW: u16 = 3;
const ROW_HEIGHT_ROW: u16 = CARDS_ROW + CARD_HEIGHT + 2;
const SWITCHES_ROW: u16 = ROW_HEIGHT_ROW + 3;
/// Smooth scrolling, motion, the minimap, the view's four and notifications.
const APPEARANCE_SWITCHES: u16 = 8;
/// The rows all of that takes, which a short window scrolls through.
const APPEARANCE_ROWS: u16 = SWITCHES_ROW + 3 * APPEARANCE_SWITCHES - 1;
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
/// A row of choices' track is a button's height, and the chosen one stands inside its line,
/// this far in from its sides, its top and its foot, in columns and rows: 2 px each way. The
/// line is set on whole pixels and the fill isn't, which at the same top and foot left the
/// fill on the line at its top and 2 px off it at its foot.
const SEGMENT_HEIGHT: f32 = 0.88;
const SEGMENT_INSET: (f32, f32, f32) = (0.18, 0.1, 0.07);
/// How far inside its row the wash behind a pointed row starts and ends, in rows: 4 of a
/// row's 64 paint units, so its middle is the row's. At 0.055 it rounded to 4 over and 3
/// under, 0.4 px low, which a button standing inside the wash showed as 3 px of it over the
/// button and 4 under.
const ROW_WASH: f32 = 0.0625;
const BUTTON_WIDTH: u16 = 12;
const TOGGLE_WIDTH: f32 = 2.6;
/// A page's title, a row tall: as large as its two rows hold with the note under it clear.
const TITLE_SIZE: u8 = 145;
const NOTE_SIZE: u8 = 90;
/// The size of what heads a column or a category, as a section's heading is anywhere.
const HEADING_SIZE: u8 = 80;
const RADIUS: u8 = 30;
/// Where the servers' rows start, from the top of the page: a heading's row under the last
/// switch and the air under it.
const SERVERS_ROW: u16 = CURSOR_ROW + 4;
/// The thumb at a page's edge when there's more of it than is in view: the least it's tall,
/// in rows, and how wide it is, in columns, as a palette's is.
const THUMB: f32 = 1.2;
const THUMB_WIDE: f32 = 0.25;
const INSTALL_WIDTH: u16 = 9;
/// Where a server's Install would only fail: Copy Command, with a column either side.
const COPY_WIDTH: u16 = 14;
/// Columns from an integration's name to its state.
const STATUS_GAP: f32 = 1.1;
/// Pixels a language server's program keeps clear before the tag after it.
const NAME_AIR: f32 = 24.0;

/// What a server's row says of installing it while it's pointed at: what Install runs, or
/// all that it needs, which starts in small letters as its tag does at rest ("needs npm, or
/// Homebrew"): pointed at, it went to a capital (the release review of 0.12.0).
fn pointed_words(plan: &Plan) -> String {
    match plan {
        Plan::Needs { says, .. } => match says.strip_prefix("Needs ") {
            Some(rest) => format!("needs {rest}"),
            None => says.clone(),
        },
        Plan::Run { says, .. } => says.clone(),
    }
}

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
    RowHeight(&'static str),
    SmoothScroll,
    Motion,
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
    CursorBlink,
    /// Empties the filter.
    ClearFilter,
    /// A command's keys, by its place in `shortcuts`: a click records new ones.
    Shortcut(usize),
    /// Gives a command its own keys back.
    ResetShortcut(usize),
    ClaudeCode,
    Codex,
    Gemini,
    Kiro,
    Ghostty,
    /// A language server to install, as an index into `SERVERS`.
    InstallServer(usize),
    /// The command that would install a server this machine can't install by itself, to copy.
    CopyServer(usize),
    /// terminal-browser, which runs a browser in the side pane.
    InstallBrowser,
    /// A command to put on the clipboard.
    CopyInstall(&'static str),
}

/// The settings as they stand, passed in so the page holds no second copy.
pub struct Values<'a> {
    pub theme: &'a str,
    pub row_height: &'a str,
    pub keymap: &'a str,
    pub smooth_scroll: bool,
    pub motion: bool,
    pub minimap: bool,
    pub view: EditorView,
    pub inlay_hints: bool,
    pub format_on_save: bool,
    pub reading_width: bool,
    pub cursor_blink: bool,
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

/// The view's switches on Appearance, between the minimap's and notifications'.
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
    /// How tall a row is.
    Height,
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

/// A line of the Keyboard page's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyLine {
    /// A category's name, over its commands.
    Heading(&'static str),
    /// A command, by its place in `shortcuts`.
    Command(usize),
    /// The empty row that parts two categories.
    Gap,
}

/// A category's heading over its commands: its name in small capitals and a rule after it
/// to the page's edge, as a group in the palette is headed. In cells the rule is a run of
/// dashes: capitals alone would be all there is to tell a heading from a name by.
fn draw_category(
    grid: &mut Grid,
    page: Rect,
    y: u16,
    category: &str,
    measure: &dyn Measure,
    theme: &Theme,
) {
    let head = Style {
        fg: theme.breadcrumb.fg,
        bold: true,
        ..theme.text
    };
    let name = category.to_uppercase();
    grid.label_at(
        page.x as f32,
        page.x,
        y,
        &name,
        head,
        page.right(),
        HEADING_SIZE,
    );
    let font = Font {
        size: HEADING_SIZE,
        bold: true,
        ..Font::default()
    };
    let from = page.x as f32 + measure.width(&name, font) + 1.0;
    let across = (f32::from(page.right()) - from).max(0.0);
    if measure.in_cells() {
        let rule = Style {
            fg: theme.border,
            ..theme.text
        };
        let dashes = "─".repeat(across as usize);
        grid.put_str(from as u16, y, &dashes, rule, page.right());
    } else {
        let rule = Bounds::cells(from, y as f32 + 0.5, across, 0.0);
        grid.hairline(rule, theme.border);
    }
}

impl Row {
    fn section(self) -> Section {
        match self {
            Row::Theme | Row::Height => Section::Appearance,
            Row::Switch(Control::FormatOnSave | Control::ReadingWidth | Control::CursorBlink) => {
                Section::Editor
            }
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
const SWITCHES: [(Control, (&str, &str)); 11] = [
    (Control::SmoothScroll, SMOOTH),
    (Control::Motion, MOTION),
    (Control::Minimap, MINIMAP),
    (Control::WordWrap, WORD_WRAP),
    (Control::StickyScroll, STICKY_SCROLL),
    (Control::BracketPairs, BRACKET_PAIRS),
    (Control::InlayHints, INLAY_HINTS),
    (Control::Notify, NOTIFY),
    (Control::FormatOnSave, FORMAT),
    (Control::ReadingWidth, READING),
    (Control::CursorBlink, CURSOR_BLINK),
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
        Control::Motion => values.motion,
        Control::Minimap => values.minimap,
        Control::WordWrap => values.view.word_wrap,
        Control::StickyScroll => values.view.sticky_scroll,
        Control::BracketPairs => values.view.bracket_pairs,
        Control::InlayHints => values.inlay_hints,
        Control::Notify => values.notify,
        Control::FormatOnSave => values.format_on_save,
        Control::ReadingWidth => values.reading_width,
        Control::CursorBlink => values.cursor_blink,
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
    pub kiro: bool,
    pub kiro_installed: bool,
    /// Whether terminal-browser is on the path.
    pub browser: bool,
    /// Whether Ghostty keeps Cmd+D, when this is Ghostty.
    pub ghostty_keeps_cmd_d: Option<bool>,
    /// For each of `SERVERS`, whether it's installed.
    pub servers: Vec<bool>,
    /// For each of `SERVERS`, what installing it does on this machine.
    pub plans: Vec<Plan>,
}

impl Found {
    pub fn look(machine: &Machine) -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self {
            claude: integrations::on_path("claude", var),
            claude_installed: integrations::claude_installed(var),
            codex: integrations::on_path("codex", var),
            codex_installed: integrations::codex_installed(var),
            gemini: integrations::on_path("gemini", var),
            gemini_installed: integrations::gemini_installed(var),
            kiro: integrations::on_path("kiro-cli", var),
            kiro_installed: integrations::kiro_installed(var),
            browser: integrations::on_path("terminal-browser", var),
            ghostty_keeps_cmd_d: integrations::ghostty_keeps_cmd_d(var),
            // Where installs put them too, which the PATH may lack.
            servers: SERVERS
                .iter()
                .map(|s| machine.find(s.command[0]).is_some())
                .collect(),
            plans: SERVERS
                .iter()
                .map(|s| crate::install::plan(s, machine))
                .collect(),
        }
    }

    /// What installing a server does on this machine.
    pub fn plan(&self, index: usize) -> Option<&Plan> {
        self.plans.get(index)
    }

    /// The button a missing server has, and its width: Install, or Copy Command where Install
    /// could only fail. One that nothing here installs and no command helps has none.
    fn server_control(&self, index: usize) -> Option<(Control, u16)> {
        if self.servers.get(index).copied().unwrap_or(false) {
            return None;
        }
        Some(match self.plan(index) {
            Some(Plan::Needs { copy: Some(_), .. }) => (Control::CopyServer(index), COPY_WIDTH),
            Some(Plan::Needs { copy: None, .. }) => return None,
            _ => (Control::InstallServer(index), INSTALL_WIDTH),
        })
    }

    /// The columns the list keeps at its rows' right for what a press does, the same on every
    /// row: as wide as the widest button any row has, so every row's tag ends at one edge. Each
    /// tag ended at its own button, 85 px further right on an Install row than on a Copy
    /// Command row, a zig-zag down the list (the release review of 0.12.0, 42-settings-needs).
    fn action_column(&self) -> u16 {
        (0..SERVERS.len())
            .filter_map(|index| self.server_control(index))
            .map(|(_, wide)| wide)
            .max()
            .unwrap_or(INSTALL_WIDTH)
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
    /// What that install is of, whose button turns while it runs.
    pub installing_what: Option<Control>,
    /// A server whose install failed, and why, which its row says until it's tried again.
    pub failed: Option<(usize, String)>,
    /// Every command and its keys, as the Keyboard section lists them.
    pub shortcuts: Vec<Shortcut>,
    /// What's typed in the filter, which narrows the rows of every page to those that fit.
    pub filter: TextBox,
    /// The first of what it found that's in view.
    pub found_top: usize,
    /// The first command in view in the Keyboard section.
    pub keys_top: usize,
    /// A command whose new keys are being pressed, and those pressed so far.
    pub recording: Option<(usize, Option<String>)>,
}

/// The part of Settings' filter that shows: where its words are painted, the cell they start
/// in, where they stop, each letter's width, and how many letters are out of sight before
/// the field's start and before the caret.
struct FilterShown {
    x: f32,
    cell: u16,
    end: u16,
    advances: std::rc::Rc<Vec<f32>>,
    hidden: usize,
    before: usize,
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
            installing_what: None,
            failed: None,
            shortcuts: Vec::new(),
            filter: TextBox::default(),
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
        if pair(ROW_HEIGHT) || ROW_HEIGHTS.iter().any(|r| fits(&[r.1])) {
            rows.push(Row::Height);
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
        if pair(CURSOR_BLINK) {
            rows.push(Row::Switch(Control::CursorBlink));
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

    /// The Keyboard page's list, top to bottom: each category's heading, its commands by
    /// name, and an empty row before the next. A row holds a command's name alone, so the
    /// category is read once, over its commands, and a name has the column to itself.
    fn key_lines(&self) -> Vec<KeyLine> {
        let mut lines = Vec::new();
        let mut last = None;
        for index in self.shown_shortcuts() {
            let category = crate::palette::category(self.shortcuts[index].name);
            if last != Some(category) {
                if last.is_some() {
                    lines.push(KeyLine::Gap);
                }
                lines.push(KeyLine::Heading(category));
                last = Some(category);
            }
            lines.push(KeyLine::Command(index));
        }
        lines
    }

    /// The lines of the list in view, each with its row. Scrolled into a category, its
    /// heading stands on the list's first row in place of the command there, so the names
    /// under it are never read without it.
    fn key_lines_in_view(&self, page: Rect) -> Vec<(u16, KeyLine)> {
        let room = page.h.saturating_sub(KEYS_FIRST_ROW) as usize;
        let lines = self.key_lines();
        let mut shown: Vec<KeyLine> = lines.into_iter().skip(self.keys_top).take(room).collect();
        if let Some(KeyLine::Command(index)) = shown.first().copied() {
            let category = crate::palette::category(self.shortcuts[index].name);
            shown[0] = KeyLine::Heading(category);
        }
        let first = page.y + KEYS_FIRST_ROW;
        let rows = shown.into_iter().enumerate();
        rows.map(|(row, line)| (first + row as u16, line)).collect()
    }

    /// The shortcut rows in view: each one's row, and its place in `shortcuts`.
    fn shortcut_rows(&self, page: Rect) -> Vec<(u16, usize)> {
        let lines = self.key_lines_in_view(page).into_iter();
        lines
            .filter_map(|(y, line)| match line {
                KeyLine::Command(index) => Some((y, index)),
                _ => None,
            })
            .collect()
    }

    /// The table's columns: where the keys start, and where the source does. The names have
    /// half the page, up to what the longest of them needs in cells: a command's title is
    /// its category and its name, and two fifths of a page cut a third of them short on a
    /// terminal without pictures.
    fn key_columns(page: Rect) -> (u16, u16) {
        let source = page.right().saturating_sub(SOURCE_WIDTH + RESET_WIDTH);
        let keys = page.x + (page.w / 2).clamp(16, 40);
        (keys, source)
    }

    /// A command as its row shows it: its name under its category's heading, or its whole
    /// title among what the filter found, where no heading stands over it. Whole, or ending
    /// in an ellipsis where the column is too narrow, never cut through a word with nothing
    /// to say so.
    fn shown_name(&self, index: usize, titled: bool, page: Rect, measure: &dyn Measure) -> String {
        let (keys_x, _) = Self::key_columns(page);
        let room = f32::from(keys_x.saturating_sub(1).saturating_sub(page.x));
        let title = self.shortcuts[index].name;
        let name = if titled {
            title
        } else {
            crate::palette::bare(title)
        };
        crate::context::elide(name, room, measure)
    }

    /// Appearance's switches, in the order the page has them.
    fn appearance_switches() -> Vec<Control> {
        let mut all = vec![Control::SmoothScroll, Control::Motion, Control::Minimap];
        all.extend(VIEW_SWITCHES);
        all.push(Control::Notify);
        debug_assert_eq!(all.len(), APPEARANCE_SWITCHES as usize);
        all
    }

    /// Where a part of Appearance stands on screen, `y` rows down the page and `rows` tall,
    /// with the page scrolled as it is. Nowhere, if any of it is past either end: half a
    /// switch isn't drawn, and nothing is drawn over the bars under the page.
    fn appearance_row(&self, page: Rect, y: u16, rows: u16) -> Option<u16> {
        let top = (self.top as u16).min(APPEARANCE_ROWS.saturating_sub(page.h));
        (y >= top && y + rows - top <= page.h).then(|| page.y + y - top)
    }

    pub fn scroll_by(&mut self, delta: isize, area: Rect) {
        if self.filtering() {
            let last = self.found_rows().len().saturating_sub(1) as isize;
            self.found_top = (self.found_top as isize + delta).clamp(0, last) as usize;
            return;
        }
        if self.section == Section::Keyboard {
            let last = self.key_lines().len().saturating_sub(1) as isize;
            self.keys_top = (self.keys_top as isize + delta).clamp(0, last) as usize;
            return;
        }
        if self.section == Section::Appearance {
            let most = APPEARANCE_ROWS.saturating_sub(Self::content(area).h) as isize;
            self.top = (self.top as isize + delta).clamp(0, most) as usize;
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
                    Some(("Copy Command", false, Control::CopyInstall(install))),
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
        // Kiro CLI where it's installed, or where its hooks are still there to take away.
        if found.kiro || found.kiro_installed {
            blocks.push(Self::kiro_block(found));
        }
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
                detail: "Ghostty keeps them for itself. Ctrl+A and Ctrl+D always work.",
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

    /// Kiro CLI's card: its hooks say what it's doing, and its prompt hook can't hand it
    /// comments, which Ask Agent sends it.
    fn kiro_block(found: &Found) -> Block {
        let (state, tone, detail, action) = if found.kiro_installed {
            (
                "Installed",
                Tone::Good,
                "Shows its work in Agents. Ask Agent sends it your comments.",
                ("Remove", false),
            )
        } else {
            (
                "Hooks not installed",
                Tone::Quiet,
                "Shows its work in Agents. Needs Kiro CLI 3.0 or later.",
                ("Install", true),
            )
        };
        Block {
            name: "Kiro CLI",
            state,
            tone,
            detail,
            action: Some((action.0, action.1, Control::Kiro)),
        }
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
                // Everything has its turn under Tab, in view or not. What isn't in view has a
                // place with no height at the page's end, which nothing is drawn in and no
                // click lands on.
                let place = |y: Option<u16>, x: u16, (w, h): (u16, u16)| match y {
                    Some(y) => Rect::new(x, y, w, h),
                    None => Rect::new(x, page.bottom(), w, 0),
                };
                let card = CARD_WIDTH.min(page.w.saturating_sub(4) / 3);
                let y = self.appearance_row(page, CARDS_ROW, CARD_HEIGHT);
                for (i, (name, _, _)) in THEMES.iter().enumerate() {
                    let x = page.x + i as u16 * (card + 2);
                    spots.push((place(y, x, (card, CARD_HEIGHT)), Control::Theme(name)));
                }
                let y = self.appearance_row(page, ROW_HEIGHT_ROW, 2);
                let start = page.right().saturating_sub(SEGMENT_WIDTH * 3);
                for (i, (name, _)) in ROW_HEIGHTS.iter().enumerate() {
                    let x = start + i as u16 * SEGMENT_WIDTH;
                    spots.push((place(y, x, (SEGMENT_WIDTH, 1)), Control::RowHeight(name)));
                }
                for (i, control) in Self::appearance_switches().into_iter().enumerate() {
                    let y = self.appearance_row(page, SWITCHES_ROW + 3 * i as u16, 2);
                    spots.push((place(y, page.x, (page.w, 2)), control));
                }
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
                let cursor = Rect::new(page.x, page.y + CURSOR_ROW, page.w, 2);
                spots.push((cursor, Control::CursorBlink));
                for (row, index) in self.server_rows(page).into_iter().enumerate() {
                    if let Some((control, width)) = self.found.server_control(index) {
                        let x = page.right().saturating_sub(width);
                        let y = page.y + SERVERS_ROW + row as u16;
                        spots.push((Rect::new(x, y, width, 1), control));
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
                Row::Height => {
                    let of = ROW_HEIGHTS.map(|r| Control::RowHeight(r.0));
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
                    if let Some((control, width)) = self.found.server_control(index) {
                        let x = page.right().saturating_sub(width);
                        spots.push((Rect::new(x, y, width, 1), control));
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

    /// What of a page is out of view, for the thumb at its edge: how many rows it has, how
    /// many are in view, how far down it's scrolled, and the row its scrolling part starts
    /// on. Appearance scrolls whole, and Editor's list of servers under what's over it.
    pub fn scrolled(&self, area: Rect) -> Option<(u16, u16, u16, u16)> {
        let page = Self::content(area);
        let (all, shown, from) = match self.section {
            _ if self.filtering() => return None,
            Section::Appearance => (APPEARANCE_ROWS, page.h, page.y),
            Section::Editor => {
                let room = page.h.saturating_sub(SERVERS_ROW);
                (SERVERS.len() as u16, room, page.y + SERVERS_ROW)
            }
            _ => return None,
        };
        (shown > 0 && all > shown).then(|| (all, shown, (self.top as u16).min(all - shown), from))
    }

    /// A thumb in the margin after the page when there's more of it than is in view: as
    /// much of what's in view as that is of the whole, and as far down. Without it nothing
    /// said a page went on, and on taller rows more pages do.
    fn draw_thumb(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        let Some((all, shown, top, from)) = self.scrolled(area) else {
            return;
        };
        let (all, shown, top) = (f32::from(all), f32::from(shown), f32::from(top));
        let tall = (shown * shown / all).max(THUMB);
        let down = (shown - tall) * top / (all - shown);
        let y = f32::from(from) + down;
        let x = Self::content(area).right() + 1;
        // In cells, half blocks down that column.
        let style = Style {
            fg: theme.scrollbar,
            painted: true,
            ..theme.text
        };
        let cells = y.round() as u16..(y + tall).round().max(y.round() + 1.0) as u16;
        for row in cells {
            grid.put_str(x, row, "▐", style, x + 1);
        }
        let left = f32::from(x) + 0.5 - THUMB_WIDE / 2.0;
        let thumb = Bounds::cells(left, y + 0.1, THUMB_WIDE, tall - 0.2);
        grid.panel(thumb, 12, Some(theme.scrollbar), None);
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
        self.reveal_focus(area);
    }

    /// Scrolls Appearance until the control the keyboard is on is in view.
    fn reveal_focus(&mut self, area: Rect) {
        if self.section != Section::Appearance || self.filtering() {
            return;
        }
        let page = Self::content(area);
        let Some((y, rows)) = self.focused(area).and_then(Self::appearance_place) else {
            return;
        };
        let most = APPEARANCE_ROWS.saturating_sub(page.h);
        let top = (self.top as u16).min(most);
        let top = if y <= CARDS_ROW {
            // The cards come with the heading over them.
            0
        } else if y < top {
            y
        } else if y + rows > top + page.h {
            y + rows - page.h
        } else {
            top
        };
        self.top = top.min(most) as usize;
    }

    /// How far down Appearance a control is, and how many rows it takes.
    fn appearance_place(control: Control) -> Option<(u16, u16)> {
        match control {
            Control::Theme(_) => Some((CARDS_ROW, CARD_HEIGHT)),
            Control::RowHeight(_) => Some((ROW_HEIGHT_ROW, 2)),
            other => {
                let switches = Self::appearance_switches();
                let at = switches.iter().position(|switch| *switch == other)?;
                Some((SWITCHES_ROW + 3 * at as u16, 2))
            }
        }
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
                let state = (hovered(*control), measure.in_cells());
                self.draw_section_tab(grid, *rect, *section, state, theme);
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
        self.draw_thumb(grid, area, theme);
        match self.section {
            Section::Appearance => {
                if let Some(y) = self.appearance_row(page, 0, 2) {
                    let at = Rect::new(page.x, y, page.w, 2);
                    let note = "Changes apply as you click.";
                    heading(grid, at, "Appearance", note, theme);
                }
                if let Some(y) = self.appearance_row(page, ROW_HEIGHT_ROW, 2) {
                    let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
                    draw_text(grid, page, (y, room), ROW_HEIGHT, theme);
                }
                // What's drawn is what has a place in view: a part the window has no room
                // for has none, and comes into view as the page scrolls.
                for (rect, control) in spots.iter().filter(|(rect, _)| rect.h > 0) {
                    match control {
                        Control::Theme(name) => {
                            let on = values.theme == *name;
                            draw_card(grid, *rect, name, (on, hovered(*control)), theme);
                        }
                        // Drawn together, as one track, after this.
                        Control::RowHeight(_) => {}
                        _ => {
                            let words = SWITCHES.iter().find(|(switch, _)| switch == control);
                            if let Some((_, words)) = words {
                                let on = switch_on(*control, values);
                                draw_toggle(grid, page, rect.y, *words, on, theme);
                            }
                        }
                    }
                }
                let heights: Vec<(Rect, &str, bool)> = spots
                    .iter()
                    .filter(|(rect, _)| rect.h > 0)
                    .filter_map(|(rect, control)| match control {
                        Control::RowHeight(name) => {
                            let label = ROW_HEIGHTS.iter().find(|r| r.0 == *name)?.1;
                            Some((*rect, label, values.row_height == *name))
                        }
                        _ => None,
                    })
                    .collect();
                draw_segments(
                    grid,
                    &heights,
                    (hover, theme.text.bg, measure.in_cells()),
                    theme,
                );
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
        let ringed = ringed.filter(|(rect, _)| self.recording.is_none() && rect.h > 0);
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
        let air = cellix_render::FIELD_AIR;
        let bounds = Bounds::cells(
            field.x as f32,
            y as f32 + air,
            field.w as f32,
            1.0 - 2.0 * air,
        );
        grid.panel(bounds, 18, Some(theme.input), Some(theme.focus));
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let icon = field.x as f32 + 0.6;
        grid.small_glyph(icon, y, Icon::Search, theme.gutter.fg, "⌕", theme.text);
        let words = grid.painted_so_far();
        let count = self.filter_count();
        let shown = self.filter_shown(area, count.as_deref(), measure);
        if let Some(count) = &count {
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
            grid.label_right(right, y, count, quiet, NOTE_SIZE);
        }
        let FilterShown { x, cell, end, .. } = shown;
        if self.filter.is_empty() {
            grid.label_at(x + 0.4, cell, y, FILTER_HINT, quiet, end, 100);
        } else {
            let from = self.filter.char_indices().nth(shown.hidden);
            let from = from.map_or(self.filter.len(), |(from, _)| from);
            let words = &self.filter[from..];
            // What's selected of what shows, as the file shows its selection.
            let selected = self.filter.selection().filter(|_| self.recording.is_none());
            let selected = selected.map(|(a, b)| (a.max(from) - from, b.max(from) - from));
            let mark = |grid: &mut Grid, drawn| {
                if let Some(range) = selected {
                    let place = (x, cell, y);
                    let pair = (measure, theme);
                    let size = (100, end, crate::palette::BOX_SELECTION_INSET);
                    crate::palette::box_selection(
                        grid,
                        place,
                        words,
                        range,
                        size,
                        theme.input,
                        pair,
                        drawn,
                    );
                }
            };
            mark(grid, false);
            grid.label_at(x, cell, y, words, theme.text, end, 100);
            mark(grid, true);
        }
        // Raised in the box, as every text box's words are.
        grid.lift_since(words, crate::measures::field_lift(measure));
        if self.recording.is_none() {
            // Where the text before it really ends, which proportional letters make short of
            // its cells; in cells, the cell typing goes into.
            let before: f32 = shown.advances[shown.hidden..shown.before].iter().sum();
            let selected = self.filter.selection().is_some();
            let nudge = crate::palette::caret_nudge_in(
                &self.filter,
                &shown.advances,
                shown.before,
                selected,
            );
            let at = match nudge {
                _ if measure.in_cells() => f32::from(cell) + before,
                // Clear of the letters either side of it, as in every box.
                Some(nudge) => (x + before + nudge).max(x),
                None => x,
            };
            grid.set_caret(at, y, 0.62);
        }
    }

    /// How many rows the filter found, said at its field's end while there's a filter.
    fn filter_count(&self) -> Option<String> {
        self.filtering().then(|| match self.found_rows().len() {
            0 => "No settings".to_string(),
            1 => "1 setting".to_string(),
            n => format!("{n} settings"),
        })
    }

    /// The part of the filter's words that shows: a filter longer than its field shows the
    /// part its caret is in, as every text box does.
    fn filter_shown(&self, area: Rect, count: Option<&str>, measure: &dyn Measure) -> FilterShown {
        let field = Self::filter_field(area);
        let x = field.x as f32 + 0.6 + 2.3;
        let cell = field.x + 4;
        let end = match count {
            Some(count) => {
                let right = field.right().saturating_sub(CLEAR_WIDTH).saturating_sub(1);
                right.saturating_sub(count.chars().count() as u16 + 1)
            }
            None => field.right().saturating_sub(1),
        };
        let advances = measure.advances(&self.filter, Font::default());
        let caret = self.filter.caret();
        let before = self.filter[..caret].chars().count().min(advances.len());
        let start = if measure.in_cells() {
            f32::from(cell)
        } else {
            x
        };
        let hidden = crate::palette::hidden_before(&advances, before, f32::from(end) - start);
        FilterShown {
            x,
            cell,
            end,
            advances,
            hidden,
            before,
        }
    }

    /// A press on the filter's field, at `x` with its fraction of a column, puts its caret at
    /// the letter pressed, and the keyboard back in it. Returns whether it was on the field.
    pub fn press_filter(&mut self, area: Rect, (x, y): (f32, u16), measure: &dyn Measure) -> bool {
        let Some(at) = self.filter_place(area, (x, y), measure, false) else {
            return false;
        };
        self.filter.set_caret(at);
        self.focus = None;
        true
    }

    /// The byte of the filter a press at `x` on row `y` stands at, when it's on the field; with
    /// `clamp`, as a drag that's left it, the nearest place in it.
    pub fn filter_place(
        &self,
        area: Rect,
        (x, y): (f32, u16),
        measure: &dyn Measure,
        clamp: bool,
    ) -> Option<usize> {
        let field = Self::filter_field(area);
        let count = self.filter_count();
        let shown = self.filter_shown(area, count.as_deref(), measure);
        if !clamp && (y != field.y || x < f32::from(field.x) || x >= f32::from(shown.end)) {
            return None;
        }
        let mut edge = if measure.in_cells() {
            f32::from(shown.cell)
        } else {
            shown.x
        };
        let mut letters = shown.hidden;
        for advance in shown.advances.iter().skip(shown.hidden) {
            if x < edge + advance / 2.0 {
                break;
            }
            edge += advance;
            letters += 1;
        }
        let at = self.filter.char_indices().nth(letters);
        Some(at.map_or(self.filter.len(), |(at, _)| at))
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
                let row: Vec<(Rect, &str, bool)> = spots
                    .iter()
                    .filter(|(r, _)| r.y == y && r.h == 1)
                    .filter_map(|(rect, control)| {
                        let (word, on) = label(*control)?;
                        Some((*rect, word, on))
                    })
                    .collect();
                draw_segments(
                    grid,
                    &row,
                    (hover, theme.text.bg, measure.in_cells()),
                    theme,
                );
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
                Row::Height => {
                    rule(grid, y);
                    let room = page.right().saturating_sub(SEGMENT_WIDTH * 3 + 1);
                    draw_text(grid, page, (y, room), ROW_HEIGHT, theme);
                    tag(grid, (ROW_HEIGHT.0, bold), (y, room), row);
                    segments(grid, y, &|control| match control {
                        Control::RowHeight(name) => {
                            let word = ROW_HEIGHTS.iter().find(|r| r.0 == name)?.1;
                            Some((word, values.row_height == name))
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
                    let shown = (hover, measure, &shared);
                    self.draw_shortcut(grid, page, (y, index, true), shown, theme);
                    let (keys_x, _) = Self::key_columns(page);
                    let name = self.shown_name(index, true, page, measure);
                    tag(
                        grid,
                        (&name, Font::default()),
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
        (hovered, cells): (bool, bool),
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
            grid.panel(Bounds::inset(rect, 0.0, ROW_WASH), 18, Some(fill), None);
        }
        // Picked, its words keep the bright text colour on its wash, as every other picked
        // row's do; they were the accent. On cells, where there's no wash, the accent says it.
        let fg = match (on, filtering && found == 0) {
            (true, _) if cells => theme.accent,
            (true, _) => theme.text.fg,
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
        let keymaps: Vec<(Rect, &str, bool)> = spots
            .iter()
            .filter_map(|(rect, control)| match control {
                Control::Keymap(name) => {
                    let label = KEYMAPS.iter().find(|k| k.0 == *name).map_or("", |k| k.1);
                    Some((*rect, label, values.keymap == *name))
                }
                _ => None,
            })
            .collect();
        draw_segments(
            grid,
            &keymaps,
            (hover, theme.text.bg, measure.in_cells()),
            theme,
        );
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
        // Midway between the heads' letters and the first row's, which sit low in their rows.
        let under = Bounds::cells(page.x as f32, y as f32 + 0.92, page.w as f32, 0.0);
        grid.hairline(under, theme.border);
        let shared = keys::sharing(&self.shortcuts);
        for (y, line) in self.key_lines_in_view(page) {
            match line {
                KeyLine::Heading(category) => {
                    draw_category(grid, page, y, category, measure, theme);
                }
                KeyLine::Command(index) => {
                    let shown = (hover, measure, &shared);
                    self.draw_shortcut(grid, page, (y, index, false), shown, theme);
                }
                KeyLine::Gap => {}
            }
        }
    }

    /// One command's row: its name, its keys or the keys being pressed for it, and whether
    /// they're its own or the person's, with a way back to its own.
    fn draw_shortcut(
        &self,
        grid: &mut Grid,
        page: Rect,
        (y, index, titled): (u16, usize, bool),
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
                y as f32 + ROW_WASH,
                page.w as f32 + 1.2,
                1.0 - ROW_WASH * 2.0,
            );
            grid.panel(wash, 18, Some(theme.page_hover), None);
        }
        let name = self.shown_name(index, titled, page, measure);
        grid.label(page.x, y, &name, theme.text, keys_x.saturating_sub(1));
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
                // What Enter and Escape do, in as many words as there's room for after the
                // keys, and in none where the keys fill the box: it was written over them.
                let shown = pressed.as_deref().unwrap_or("Press the keys");
                let taken = keys_x as f32 + measure.width(shown, Font::default()) + 1.0;
                let hints: &[&str] = match pressed {
                    Some(_) => &["Enter saves · Esc cancels", "Enter saves"],
                    None => &["Esc cancels"],
                };
                let note = Font {
                    size: NOTE_SIZE,
                    ..Font::default()
                };
                let end = keys_end.saturating_sub(1);
                let fits = |hint: &&&str| taken + measure.width(hint, note) <= f32::from(end);
                if let Some(hint) = hints.iter().find(fits) {
                    grid.label_right(end, y, hint, quiet, NOTE_SIZE);
                }
            }
            _ => {
                let mut x = keys_x;
                if shortcut.keys.is_empty() {
                    grid.label(x, y, "—", quiet, keys_end);
                }
                for key in &shortcut.keys {
                    let width = measure.width(key, Font::default());
                    let cap = Bounds::cells(x as f32 - 0.6, y as f32 + 0.12, width + 1.2, 0.76);
                    grid.panel(cap, 24, Some(theme.activity.bg), Some(theme.popup_border));
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
            "How files save, how pages are set, the cursor, and the languages.",
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
        let saves: Vec<(Rect, &str, bool)> = spots
            .iter()
            .filter_map(|(rect, control)| match control {
                Control::AutoSave(name) => {
                    let label = AUTO_SAVES.iter().find(|a| a.0 == *name).map_or("", |a| a.1);
                    Some((*rect, label, values.auto_save == *name))
                }
                _ => None,
            })
            .collect();
        draw_segments(
            grid,
            &saves,
            (hover, theme.text.bg, measure.in_cells()),
            theme,
        );
        let y = page.y + READING_ROW;
        let above = Bounds::cells(page.x as f32, y as f32 - 0.5, page.w as f32, 0.0);
        grid.hairline(above, theme.border);
        draw_toggle(grid, page, y, READING, values.reading_width, theme);
        // Its line over it is the toggle's own.
        let y = page.y + CURSOR_ROW;
        draw_toggle(grid, page, y, CURSOR_BLINK, values.cursor_blink, theme);
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
        // Ending where the rows' words at their ends do (`words_end`).
        let (end, cell) = words_end(page, measure);
        grid.label_right_at(end, cell, line_y, &found, plain, NOTE_SIZE);
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
            // The whole row, so a button on it has its 8 px and more over and under: inside
            // a wash set in from the row, Install stood 5 px under its top.
            let wash = Bounds::cells(page.x as f32 - 0.6, y as f32, page.w as f32 + 1.2, 1.0);
            grid.panel(wash, 18, Some(theme.page_hover), None);
        }
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        grid.label(page.x, y, server.languages[0], theme.text, page.x + detail);
        let found = self.found.servers.get(index).copied().unwrap_or(false);
        let control = self.found.server_control(index);
        // Every row's tag ends at one edge, a column before the list's column for what a
        // press does (`action_column`).
        let end = page.right().saturating_sub(self.found.action_column() + 1);
        // The program always, and what installing it does here in a slot of its own before
        // the button: at rest a word or two, the tool or where it's from or what it needs;
        // pointed at, the whole of it, in the same quiet colour, and what it needs still in
        // small letters, as at rest; after a failure, why.
        let program = server.command[0];
        let font = Font {
            size: NOTE_SIZE,
            ..Font::default()
        };
        let plan = self.found.plan(index).filter(|_| !found);
        let failed = self.failed.as_ref().filter(|(at, _)| *at == index);
        let error = Style {
            fg: theme.error,
            ..theme.text
        };
        let installing = self.installing.is_some()
            && self.installing_what == Some(Control::InstallServer(index));
        let tag = plan.map(Plan::tag);
        let said = match (plan, failed) {
            _ if installing => None,
            (Some(plan), _) if hovered => Some((pointed_words(plan), quiet)),
            // A failure stays on its row, where it was asked for, until it's tried again.
            (_, Some((_, why))) => {
                let why = why.split_once(": ").map_or(why.as_str(), |(_, why)| why);
                Some((format!("Couldn't: {why}"), error))
            }
            (Some(_), _) => tag.clone().map(|tag| (tag, quiet)),
            _ => None,
        };
        // The program's name stops `NAME_AIR` before the tag the row has at rest, cut with a
        // "…" where it's longer, so the two never read as one phrase: "…-wrapper" ended 10 px
        // before "needs Homebrew", both in one grey (the release review of 0.12.0). Pointed
        // at, it keeps that place, and the longer words have what's left after it.
        let air = crate::measures::columns(NAME_AIR, measure).unwrap_or(1.0);
        let start = f32::from(page.x + detail);
        let resting = match (&said, &tag) {
            (Some((words, _)), _) if !hovered => Some(words.clone()),
            (_, Some(tag)) => Some(tag.clone()),
            _ => None,
        };
        let name_room = match &resting {
            Some(words) => f32::from(end) - measure.width(words, font) - air - start,
            None => f32::from(end) - start,
        };
        let name = crate::context::elide(program, name_room.max(0.0), measure);
        grid.label(page.x + detail, y, &name, quiet, end);
        let named = start + measure.width(&name, Font::default()) + air;
        if let Some((words, style)) = said {
            let words = cut_middle(&words, f32::from(end) - named, measure, font);
            grid.label_right(end, y, &words, style, NOTE_SIZE);
        }
        // What ends the row ends where a button's label does, at rest and pointed at alike.
        let (words, cell) = words_end(page, measure);
        if found {
            let state = Style {
                fg: theme.git_added,
                ..theme.text
            };
            grid.label_right_at(words, cell, y, "Found", state, NOTE_SIZE);
            return;
        }
        // Being installed, where its button was: a spinner and what it's doing.
        if installing {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.text
            };
            let font = Font {
                size: NOTE_SIZE,
                ..Font::default()
            };
            let at = words - measure.width("Installing", font) - 1.6;
            grid.spinner(at, y, crate::measures::WAIT_SPIN, theme.accent, theme.text);
            grid.label_right_at(words, cell, y, "Installing", quiet, NOTE_SIZE);
            return;
        }
        let Some((control, _)) = control else {
            return;
        };
        let spot = spots.iter().find(|(_, c)| *c == control);
        if let Some((rect, _)) = spot {
            // Install, or where it could only fail, the command to copy, a quiet button.
            let (label, main) = match control {
                Control::CopyServer(_) => ("Copy Command", false),
                _ => ("Install", true),
            };
            // Words in the accent until its row is pointed at: twenty chips in a column
            // stood a few pixels apart, a wall beside the list.
            let (x, wide) = button_place(*rect, label, measure);
            let cells = format!("{:^width$}", label, width = wide as usize);
            if hovered {
                let (fill, fg) = if main {
                    (theme.button, theme.button_text)
                } else {
                    let on_it = hover.is_some_and(|h| rect.contains(h.0, h.1));
                    (theme.secondary(theme.page_hover, on_it), theme.text.fg)
                };
                let style = Style { fg, ..theme.text };
                if measure.in_cells() {
                    // A column either side of its word, which ends where it does at rest.
                    grid.row_button((x, rect.y), &cells, label, fill, style);
                } else {
                    // Its label where it stands at rest, and the chip behind it reaching
                    // `BUTTON_ROOM` past it either side, to the page's end, in the middle of
                    // the row's wash: pointing adds the fill and moves nothing.
                    let font = Font {
                        size: NOTE_SIZE,
                        ..Font::default()
                    };
                    let room = crate::measures::BUTTON_ROOM;
                    let left = words - measure.width(label, font) - room;
                    grid.row_chip((left, words + room), rect.y, fill);
                    grid.label_right_at(words, cell, y, label, style, NOTE_SIZE);
                }
            } else {
                // Its word where Found stands on the rows beside it.
                let word = Style {
                    fg: theme.accent,
                    ..theme.text
                };
                grid.label_right_at(words, cell, y, label, word, NOTE_SIZE);
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

    /// Where the cards' buttons sit on a page already laid out: at the card's right, as wide
    /// as a button or as its words and a column either side, whichever is wider. At twelve
    /// columns, Copy Install Command ran past its chip at both ends.
    fn controls_at(&self, page: Rect) -> Vec<(Rect, Control)> {
        let blocks = self.blocks();
        let placed = blocks.iter().enumerate();
        placed
            .filter_map(|(index, block)| {
                let (label, _, control) = block.action?;
                let wide = BUTTON_WIDTH.max(label.chars().count() as u16 + 2);
                let x = page.right().saturating_sub(wide + 2);
                let y = page.y + 3 + index as u16 * BLOCK_STEP;
                Some((Rect::new(x, y, wide, 2), control))
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
    grid.label_at(x, page.x, page.y + 1, note, quiet, page.right(), NOTE_SIZE);
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
    // Small words, as the design has what explains a setting: the longest of them fits a
    // window of 120 columns that way.
    let x = page.x;
    grid.label_at(f32::from(x), x, y + 1, detail, quiet, room, NOTE_SIZE);
}

/// What a command's row is drawn with: the pointer, the text's measure, and how many
/// commands each set of keys is bound to, which marks keys two commands share.
type Shown<'a> = (
    Option<(u16, u16)>,
    &'a dyn Measure,
    &'a std::collections::HashMap<String, usize>,
);

/// A row of choices, one of them chosen: one track with a line round it and the ground showing
/// through, the chosen one filled with the accent inside it, and the others plain words that
/// wash under the pointer. Each was a box of its own in the frame's colour, so the ones not
/// chosen read as more text boxes (the design review of 0.11.0).
pub(crate) fn draw_segments(
    grid: &mut Grid,
    segments: &[(Rect, &str, bool)],
    (hover, ground, cells): (Option<(u16, u16)>, Rgb, bool),
    theme: &Theme,
) {
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return;
    };
    let (row, left, right) = (first.0.y, first.0.x, last.0.right());
    let top = f32::from(row) + (1.0 - SEGMENT_HEIGHT) / 2.0;
    let track = Bounds::cells(
        f32::from(left),
        top,
        f32::from(right - left),
        SEGMENT_HEIGHT,
    );
    // The line a text box without the keyboard has: a menu's was 1.34 to 1 on a dialog.
    grid.panel(track, 22, None, Some(theme.field_line()));
    for (rect, label, on) in segments {
        let hovered = !on && hover.is_some_and(|h| rect.contains(h.0, h.1));
        let fill = if *on {
            Some(theme.button)
        } else if hovered {
            Some(theme.quiet_button(ground))
        } else {
            None
        };
        let fg = if *on {
            theme.button_text
        } else {
            theme.text.fg
        };
        // The cell tier has no paint, so there the chosen one is the accent's cells; where
        // there's paint the cells keep the ground, or they'd show as a square block.
        let style = Style {
            fg,
            bg: fill.filter(|_| cells).unwrap_or(ground),
            painted: true,
            ..theme.text
        };
        let words = format!("{label:^width$}", width = rect.w as usize);
        grid.put_str(rect.x, rect.y, &words, style, rect.right());
        if let Some(fill) = fill {
            // Inside the track's line, with as much of the track showing on every side.
            let inner = Bounds::cells(
                f32::from(rect.x) + SEGMENT_INSET.0,
                top + SEGMENT_INSET.1,
                f32::from(rect.w) - SEGMENT_INSET.0 * 2.0,
                SEGMENT_HEIGHT - SEGMENT_INSET.1 - SEGMENT_INSET.2,
            );
            grid.panel(inner, 16, Some(fill), None);
        }
        let middle = f32::from(rect.x) + f32::from(rect.w) / 2.0;
        grid.button_words(middle, row, label, fg);
    }
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
    // The sidebar in what's raised, a menu's colour: in the frame's or the sheet's, as it
    // really is, graphite's three were 1.07 to 1 apart at this size and the panels didn't show.
    grid.panel(inner(x + 1.4, w * 0.26), 12, Some(sample.popup), None);
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
        fg: theme.popup_dim,
        bg: theme.popup,
        ..theme.text
    };
    // Small words, as what explains a setting is. Short of the button's place, and cut with
    // an ellipsis, not mid-word at its edge: `elide` measures words at their full size, so
    // it's given the room these take at theirs.
    let room = page.right().saturating_sub(BUTTON_WIDTH + 3);
    let wide = f32::from(room.saturating_sub(x)) * 100.0 / f32::from(NOTE_SIZE);
    let detail = crate::context::elide(detail, wide, measure);
    grid.label_at(f32::from(x), x, top + 1, &detail, quiet, room, NOTE_SIZE);
}

/// As many columns of `rect` as a button with these words takes, from its right end: the
/// place it's clicked stays `rect`, and the chip is no wider than its words need.
/// Words cut in their middle to fit `room` columns, as a path is, so both ends say what they
/// say: "Fetches marksman-linux…into ~/.local/bin".
fn cut_middle(words: &str, room: f32, measure: &dyn Measure, font: Font) -> String {
    if measure.width(words, font) <= room {
        return words.to_string();
    }
    let letters: Vec<char> = words.chars().collect();
    for kept in (0..letters.len()).rev() {
        let (head, tail) = (kept.div_ceil(2), kept / 2);
        let cut: String = letters[..head]
            .iter()
            .chain(['…'].iter())
            .chain(&letters[letters.len() - tail..])
            .collect();
        if measure.width(&cut, font) <= room {
            return cut;
        }
    }
    String::new()
}

/// Where the words at a language server's row's end end, between cells and in cells: Found,
/// what's being installed, a button's label, and the count on the list's heading. A button's
/// label stands there at rest and pointed at, `BUTTON_ROOM` in from the page's end where its
/// chip ends, so pointing at a row only adds the chip: centred in a chip of whole columns,
/// Install moved 26 px left and Copy Command 31 (the release review of 0.12.0, 42-settings-
/// needs). On cells, a column in, where a button's word has a column of its chip after it.
fn words_end(page: Rect, measure: &dyn Measure) -> (f32, u16) {
    let cell = page.right().saturating_sub(1);
    if measure.in_cells() {
        return (f32::from(cell), cell);
    }
    (f32::from(page.right()) - crate::measures::BUTTON_ROOM, cell)
}

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
    let fill = if primary {
        theme.button
    } else {
        theme.secondary(theme.popup, hovered)
    };
    let fg = if primary {
        theme.button_text
    } else {
        theme.text.fg
    };
    let style = Style { fg, ..theme.text };
    let (x, wide) = button_place(rect, label, measure);
    // As many of its letters as its width has cells: more would widen the chip past it,
    // which stood Copy Install Command on its card's edge.
    let held: String = label.chars().take(wide as usize).collect();
    let cells = format!("{held:^width$}", width = wide as usize);
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
        // Tall enough for all of Appearance, whose last switch a short window scrolls to
        // (`row_height_is_chosen_in_settings` has that): 41 rows since Motion joined it.
        let area = Rect { h: 44, ..AREA };
        let mut settings = page(Section::Appearance);
        let spots = settings.controls(area);
        assert!(spots.contains(&(spots[4].0, Control::Theme("auto"))));
        assert!(spots.iter().any(|(_, c)| *c == Control::Motion));
        for (rect, control) in &spots {
            assert_eq!(settings.control_at(area, rect.x, rect.y), Some(*control));
        }
        for _ in 0..spots.len() {
            settings.step(area, 1);
        }
        assert_eq!(settings.focused(area), spots.last().map(|s| s.1));
        settings.step(area, 1);
        assert_eq!(settings.focused(area), Some(spots[0].1), "and it wraps");
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

    /// A card's button holds its words where there are pictures: Copy Install Command ran past
    /// a chip of twelve columns, its first and last letters outside it.
    #[test]
    fn a_cards_button_holds_its_words() {
        let settings = page(Section::Integrations);
        let measure = cellix_render::FontMeasure::new(17, 50).with_type_row(36);
        let theme = Theme::default();
        let values = Values {
            theme: "dark",
            row_height: "airy",
            keymap: "default",
            smooth_scroll: false,
            motion: false,
            minimap: false,
            view: EditorView::default(),
            inlay_hints: false,
            format_on_save: false,
            reading_width: false,
            cursor_blink: false,
            auto_save: "off",
            notify: false,
        };
        let mut grid = Grid::new(AREA.w, AREA.h, theme.text);
        settings.draw(&mut grid, AREA, &values, (None, &measure), &theme);
        let label = "Copy Command";
        let unit = cellix_render::UNIT as f32;
        let middle = grid.paint.iter().find_map(|p| match p {
            cellix_render::Paint::Text {
                x,
                text,
                centred: true,
                ..
            } if text == label => Some(*x as f32 / unit),
            _ => None,
        });
        let middle = middle.expect("the button's words");
        let font = cellix_render::Font {
            size: 90,
            ..cellix_render::Font::default()
        };
        let half = measure.width(label, font) / 2.0;
        let chip = grid.paint.iter().find_map(|p| match p {
            cellix_render::Paint::Panel {
                bounds,
                fill: Some(_),
                ..
            } if bounds.h < cellix_render::UNIT
                && (bounds.x as f32) / unit <= middle
                && middle <= (bounds.x + bounds.w) as f32 / unit =>
            {
                Some((bounds.x as f32 / unit, (bounds.x + bounds.w) as f32 / unit))
            }
            _ => None,
        });
        let (left, right) = chip.expect("its chip");
        assert!(
            left <= middle - half,
            "its words start {left} before {}",
            middle - half
        );
        assert!(
            right >= middle + half,
            "and end {right} after {}",
            middle + half
        );
    }

    /// Integrations lists Kiro CLI with the other agents only where `kiro-cli` is installed,
    /// or where its hooks are there to remove, with what its button does there.
    #[test]
    fn kiro_cli_is_listed_where_it_is_installed() {
        let mut settings = page(Section::Integrations);
        let kiro = |s: &SettingsPage| {
            let blocks = s.blocks();
            let block = blocks.iter().find(|b| b.name == "Kiro CLI")?;
            let offered = s.controls(AREA).iter().any(|c| c.1 == Control::Kiro);
            assert!(offered, "its button is where a click lands");
            Some((block.state, block.action.map(|a| a.0)))
        };
        assert_eq!(kiro(&settings), None, "not here: not listed");
        settings.found.kiro = true;
        assert_eq!(
            kiro(&settings),
            Some(("Hooks not installed", Some("Install")))
        );
        // After Gemini CLI, with the other agents.
        let names: Vec<&str> = settings.blocks().iter().map(|b| b.name).collect();
        let at = |name| names.iter().position(|n| *n == name).unwrap();
        assert_eq!(at("Kiro CLI"), at("Gemini CLI") + 1, "{names:?}");
        settings.found.kiro_installed = true;
        assert_eq!(kiro(&settings), Some(("Installed", Some("Remove"))));
        settings.found.kiro = false;
        assert_eq!(
            kiro(&settings),
            Some(("Installed", Some("Remove"))),
            "gone from the PATH, its hooks can still be removed"
        );
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
