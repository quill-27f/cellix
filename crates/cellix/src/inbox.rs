//! What's waiting for the person, in every workspace: an agent asking permission, a plan to
//! review, an agent done with its turn, an answer to a comment, a test run that failed. Each
//! workspace keeps its own list as things happen and drops what's been dealt with. The inbox
//! in the side pane shows them all, and while Cellix isn't the window in front the new ones
//! become the terminal's own notifications, no more often than `GAP`.

use std::time::{Duration, Instant};

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::theme::Theme;
use crate::threads::{SMALL, button_cells, wrap};
use crate::toolbar::draw_icon;

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An agent asks something only the person can answer, such as leave to run a command.
    Permission,
    /// A plan waits for review; the agent waits with it.
    Plan,
    /// An agent finished its turn.
    Finished,
    /// An agent answered one of the person's comments.
    Reply,
    /// A test run failed.
    TestFailed,
}

impl Kind {
    fn icon(self) -> Icon {
        match self {
            Self::Permission => Icon::Question,
            Self::Plan => Icon::ListTask,
            Self::Finished => Icon::CircleCheck,
            Self::Reply => Icon::Comment,
            Self::TestFailed => Icon::CircleX,
        }
    }

    fn colour(self, theme: &Theme) -> Rgb {
        match self {
            Self::Permission => theme.attention,
            Self::Plan | Self::Reply => theme.accent,
            Self::Finished => theme.git_added,
            Self::TestFailed => theme.deleted(),
        }
    }

    /// What a click on it does, said on the row under the pointer.
    fn action(self) -> &'static str {
        match self {
            Self::Permission => "Show Prompt",
            Self::Plan => "Open Plan",
            Self::Finished => "Show Session",
            Self::Reply => "Open Thread",
            Self::TestFailed => "See Failures",
        }
    }
}

/// Where a click on it goes, in its workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An agent's session, by the agent's name.
    Session(String),
    Plan,
    /// A comment, by its number.
    Thread(u32),
    Tests,
}

/// One thing waiting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub kind: Kind,
    /// Stays the same while it's the same thing, so news about it replaces it.
    pub key: String,
    /// What happened, as a notification says it.
    pub title: String,
    /// Who or where, on a second line.
    pub detail: String,
    pub since: Instant,
    pub target: Target,
}

/// A workspace's list, newest last, and what's new since the notifier last looked.
#[derive(Default)]
pub struct Inbox {
    pub items: Vec<Waiting>,
    fresh: Vec<Waiting>,
}

impl Inbox {
    /// Adds `item`, in place of anything with its key, as news to tell.
    pub fn note(&mut self, item: Waiting) {
        self.items.retain(|w| w.key != item.key);
        self.fresh.retain(|w| w.key != item.key);
        self.fresh.push(item.clone());
        self.items.push(item);
    }

    /// Takes away what's been dealt with, before it's told if it hasn't been yet.
    pub fn drop_key(&mut self, key: &str) {
        self.items.retain(|w| w.key != key);
        self.fresh.retain(|w| w.key != key);
    }

    pub fn drop_kind(&mut self, kind: Kind) {
        self.items.retain(|w| w.kind != kind);
        self.fresh.retain(|w| w.kind != kind);
    }

    /// What's new since this was last asked.
    pub fn take_fresh(&mut self) -> Vec<Waiting> {
        std::mem::take(&mut self.fresh)
    }

    pub fn has(&self, key: &str) -> bool {
        self.items.iter().any(|w| w.key == key)
    }
}

/// How long the terminal is left alone after a notification. What comes meanwhile is told in
/// one when it's over, so a burst is one notification rather than a stack of them.
pub const GAP: Duration = Duration::from_secs(10);

/// Turns news into notifications, at most one per `GAP`.
#[derive(Default)]
pub struct Notifier {
    last: Option<Instant>,
    /// What waits to be told, with the workspace it's in.
    pending: Vec<(String, String)>,
}

impl Notifier {
    pub fn push(&mut self, workspace: &str, what: &str) {
        self.pending.push((workspace.to_string(), what.to_string()));
    }

    /// Forgets what wasn't told yet: the person is back and can see it.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// The notification to send now, as its title and body, if one is due.
    pub fn due(&mut self, now: Instant) -> Option<(String, String)> {
        if self.pending.is_empty() || self.last.is_some_and(|at| now - at < GAP) {
            return None;
        }
        self.last = Some(now);
        let pending = std::mem::take(&mut self.pending);
        if let [(workspace, what)] = pending.as_slice() {
            return Some((format!("Cellix · {workspace}"), what.clone()));
        }
        let mut places: Vec<&str> = Vec::new();
        for (workspace, _) in &pending {
            if !places.contains(&workspace.as_str()) {
                places.push(workspace);
            }
        }
        let places = match places.as_slice() {
            [one] => one.to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        };
        Some((
            "Cellix".to_string(),
            format!("{} things waiting in {places}", pending.len()),
        ))
    }
}

/// One row of the inbox: what's waiting, and the workspace it waits in.
#[derive(Clone, Debug)]
pub struct Listed {
    /// The workspace's folder, which a click goes to.
    pub root: std::path::PathBuf,
    /// What the workspace is called.
    pub place: String,
    /// Whether it's the workspace in front.
    pub here: bool,
    pub item: Waiting,
}

/// Newest first.
pub fn sorted(mut rows: Vec<Listed>) -> Vec<Listed> {
    rows.sort_by_key(|row| std::cmp::Reverse(row.item.since));
    rows
}

/// What a click in the inbox lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A row, by its place in the list: go there.
    Open(usize),
    /// The × on a row.
    Dismiss(usize),
    ClearFinished,
    Settings,
    StartAgent,
}

/// Columns kept clear at each side, and rows each row takes with the air under it.
const SIDE: u16 = 2;
const ROW: u16 = 3;
/// Rows above the first row: the count, and air under it.
const HEAD: u16 = 2;
const CLEAR_FINISHED: &str = "Clear Finished";
const SETTINGS: &str = "Notification Settings";
const START: &str = "Start an Agent";
/// How tall a row's mark is, in rows: the size of the icons at the top right.
const MARK: f32 = 0.62;

fn small() -> Font {
    Font {
        size: SMALL,
        ..Font::default()
    }
}

/// How long ago, said briefly.
fn age(since: Instant, now: Instant) -> String {
    let seconds = now.saturating_duration_since(since).as_secs();
    match seconds {
        0..60 => "now".to_string(),
        60..3600 => format!("{} min", seconds / 60),
        3600..86400 => format!("{} h", seconds / 3600),
        _ => format!("{} d", seconds / 86400),
    }
}

/// The count at the top, saying across how many workspaces.
fn summary(rows: &[Listed]) -> String {
    let mut places: Vec<&str> = rows.iter().map(|r| r.place.as_str()).collect();
    places.sort_unstable();
    places.dedup();
    match places.len() {
        0 | 1 => format!("{} waiting · newest first", rows.len()),
        n => format!("{} waiting in {n} workspaces · newest first", rows.len()),
    }
}

/// Where Clear Finished stands, when anything finished is listed.
fn clear_button(area: Rect, rows: &[Listed], measure: &dyn Measure) -> Option<(String, u16)> {
    if !rows.iter().any(|r| r.item.kind == Kind::Finished) {
        return None;
    }
    let cells = button_cells(CLEAR_FINISHED, measure);
    // Its padding stands past the column, so its words end where the ages do.
    let x = area
        .right()
        .saturating_sub(SIDE + cells.chars().count() as u16)
        + 1;
    Some((cells, x))
}

/// The row on screen of list row `index`, scrolled `top` rows down.
fn row_y(area: Rect, index: usize, top: usize) -> Option<u16> {
    let at = (index.checked_sub(top)? as u16).checked_mul(ROW)?;
    let y = area.y + 1 + HEAD + at;
    (y + 1 < area.bottom()).then_some(y)
}

/// The furthest the list scrolls: its last row the last that shows.
pub fn last_top(area: Rect, rows: usize) -> usize {
    let fit = (0..rows)
        .take_while(|index| row_y(area, *index, 0).is_some())
        .count();
    rows.saturating_sub(fit)
}

/// Where the empty inbox's buttons stand: Notification Settings, then Start an Agent.
fn empty_buttons(area: Rect, measure: &dyn Measure) -> [(String, u16, u16); 2] {
    let lines = empty_lines(area, measure);
    let y = area.y + 2 + lines.len() as u16 + 1;
    let first = button_cells(SETTINGS, measure);
    let x = area.x + SIDE;
    let second_x = x + first.chars().count() as u16 + 1;
    let second = button_cells(START, measure);
    [(first, x, y), (second, second_x, y)]
}

/// What the empty inbox says, as lines: `None` is a rule, and a kind is a row with its mark.
fn empty_lines(area: Rect, measure: &dyn Measure) -> Vec<(Option<Kind>, String, bool)> {
    let width = area.w.saturating_sub(SIDE * 2) as f32;
    let mut lines = vec![(None, "Nothing is waiting for you".to_string(), true)];
    lines.push((None, String::new(), false));
    for line in wrap(
        "Whatever needs you in any workspace shows here, newest first:",
        width,
        measure,
    ) {
        lines.push((None, line, false));
    }
    lines.push((None, String::new(), false));
    for (kind, words) in [
        (
            Kind::Permission,
            "An agent asks permission to run something",
        ),
        (Kind::Plan, "A plan is ready for you to review"),
        (Kind::Reply, "An agent answers one of your comments"),
        (Kind::TestFailed, "A test run fails"),
        (Kind::Finished, "An agent finishes its turn"),
    ] {
        lines.push((Some(kind), words.to_string(), false));
    }
    lines.push((None, String::new(), false));
    lines.push((None, "\u{2500}".to_string(), false));
    lines.push((None, String::new(), false));
    let told = "While Cellix isn't the window in front, these reach your terminal as notifications too, through tmux and SSH.";
    for line in wrap(told, width, measure) {
        lines.push((None, line, false));
    }
    lines
}

pub fn hit(
    area: Rect,
    rows: &[Listed],
    top: usize,
    (x, y): (u16, u16),
    measure: &dyn Measure,
) -> Option<Hit> {
    if rows.is_empty() {
        let [(settings, sx, sy), (start, tx, _)] = empty_buttons(area, measure);
        let on =
            |cells: &String, at: u16| y == sy && x >= at && x < at + cells.chars().count() as u16;
        return if on(&settings, sx) {
            Some(Hit::Settings)
        } else if on(&start, tx) {
            Some(Hit::StartAgent)
        } else {
            None
        };
    }
    if let Some((cells, bx)) = clear_button(area, rows, measure)
        && y == area.y + 1
        && x >= bx
        && x < bx + cells.chars().count() as u16
    {
        return Some(Hit::ClearFinished);
    }
    let right = area.right().saturating_sub(SIDE);
    let top = top.min(last_top(area, rows.len()));
    (top..rows.len()).find_map(|index| {
        let row = row_y(area, index, top)?;
        if y != row && y != row + 1 {
            return None;
        }
        // The × stands at the end of the first line.
        if y == row && x + 3 >= right && x < right {
            Some(Hit::Dismiss(index))
        } else {
            Some(Hit::Open(index))
        }
    })
}

/// Draws the inbox into `area`, the side pane below its tabs.
pub fn draw(
    grid: &mut Grid,
    area: Rect,
    (rows, top, now): (&[Listed], usize, Instant),
    hover: Option<(u16, u16)>,
    (measure, theme): (&dyn Measure, &Theme),
) {
    let body = theme.sidebar;
    grid.fill(area, body);
    let dim = Style {
        fg: theme.gutter.fg,
        ..body
    };
    let left = area.x + SIDE;
    let right = area.right().saturating_sub(SIDE);
    if rows.is_empty() {
        draw_empty(grid, area, hover, (measure, theme));
        return;
    }
    let head = area.y + 1;
    let mut end = right;
    if let Some((cells, x)) = clear_button(area, rows, measure) {
        let width = cells.chars().count() as u16;
        let on_it = hover.is_some_and(|h| h.1 == head && h.0 >= x && h.0 < x + width);
        // A quiet button, as every one that isn't the main one is, lighter when pointed at.
        let quiet = theme.quiet_button(body.bg);
        let fill = if on_it {
            theme.quiet_button(quiet)
        } else {
            quiet
        };
        let style = Style { bg: fill, ..body };
        ground(grid, x, head, width, body.bg);
        grid.button(x, head, &cells, CLEAR_FINISHED, fill, style);
        end = x.saturating_sub(1);
    }
    grid.label_at(left as f32, left, head, &summary(rows), dim, end, SMALL);
    let text_x = left + 3;
    let mut last = head;
    let top = top.min(last_top(area, rows.len()));
    for (index, row) in rows.iter().enumerate().skip(top) {
        let Some(y) = row_y(area, index, top) else {
            break;
        };
        last = y + 1;
        let item = &row.item;
        let hovered = hover
            .is_some_and(|h| h.0 >= area.x && h.0 < area.right() && (h.1 == y || h.1 == y + 1));
        if hovered {
            let wash = Bounds::cells(
                area.x as f32 + 1.0,
                y as f32 - 0.3,
                area.w.saturating_sub(2) as f32,
                2.6,
            );
            grid.panel(wash, 22, Some(theme.row_hover), None);
        }
        let ground = Style {
            bg: if hovered { theme.row_hover } else { body.bg },
            ..body
        };
        let quiet = Style {
            fg: theme.gutter.fg,
            ..ground
        };
        let mark = Rect::new(left.saturating_sub(1), y, 3, 1);
        draw_icon(
            grid,
            (item.kind.icon(), MARK),
            mark,
            item.kind.colour(theme),
            ground,
        );
        // At the end of the first line: how long ago, or under the pointer, what a click does
        // and an × to let it go.
        let title_end = if hovered {
            grid.glyph(
                right as f32 - 1.5,
                y,
                Icon::Close,
                theme.gutter.fg,
                "×",
                quiet,
            );
            // On a chip, so it reads as what a click does rather than a caption.
            let action = item.kind.action();
            let words = measure.width(action, small());
            let x = right as f32 - 3.0 - words;
            let chip = Bounds::cells(x - 1.0, y as f32 + 0.05, words + 2.0, 0.9);
            grid.panel(chip, 30, Some(theme.tint), None);
            let on_chip = Style {
                bg: theme.tint,
                ..ground
            };
            grid.label_at(
                x,
                x.floor() as u16,
                y,
                action,
                on_chip,
                right.saturating_sub(2),
                SMALL,
            );
            ((x - 1.0).floor() as u16).saturating_sub(1)
        } else {
            let when = age(item.since, now);
            let x = right as f32 - measure.width(&when, small());
            grid.label_at(x, x.floor() as u16, y, &when, quiet, right, SMALL);
            (x.floor() as u16).saturating_sub(1)
        };
        grid.label(text_x, y, &item.title, ground, title_end);
        // Where it is, then what about. The workspace in front is said quietly.
        let place = Style {
            fg: if row.here { theme.gutter.fg } else { body.fg },
            ..ground
        };
        let after = grid.label_at(
            text_x as f32,
            text_x,
            y + 1,
            &row.place,
            place,
            right,
            SMALL,
        );
        if !item.detail.is_empty() {
            let x = text_x as f32 + measure.width(&row.place, small());
            let detail = format!(" · {}", item.detail);
            grid.label_at(x, after, y + 1, &detail, quiet, right, SMALL);
        }
    }
    let hint = area.bottom().saturating_sub(2);
    if hint > last + 2 {
        let words = "Click one to go there and open it. Each leaves the list once dealt with.";
        let lines = wrap(words, right.saturating_sub(left) as f32, measure);
        let start = hint.saturating_sub(lines.len().saturating_sub(1) as u16);
        for (i, line) in lines.iter().enumerate() {
            grid.label_at(left as f32, left, start + i as u16, line, dim, right, SMALL);
        }
    }
}

fn draw_empty(
    grid: &mut Grid,
    area: Rect,
    hover: Option<(u16, u16)>,
    (measure, theme): (&dyn Measure, &Theme),
) {
    let body = theme.sidebar;
    let dim = Style {
        fg: theme.gutter.fg,
        ..body
    };
    let left = area.x + SIDE;
    let right = area.right().saturating_sub(SIDE);
    for (y, (kind, text, heading)) in (area.y + 2..).zip(empty_lines(area, measure)) {
        if y + 1 >= area.bottom() {
            return;
        }
        match kind {
            Some(kind) => {
                let mark = Rect::new(left.saturating_sub(1), y, 3, 1);
                draw_icon(grid, (kind.icon(), MARK), mark, kind.colour(theme), body);
                grid.label_at(left as f32 + 3.0, left + 3, y, &text, dim, right, SMALL);
            }
            None if text == "\u{2500}" => {
                let rule = Bounds::cells(left as f32, y as f32 + 0.5, (right - left) as f32, 0.0);
                grid.hairline(rule, theme.border);
            }
            None if heading => {
                let bold = Style { bold: true, ..body };
                grid.glyph(left as f32, y, Icon::Bell, theme.gutter.fg, "◬", dim);
                grid.label(left + 3, y, &text, bold, right);
            }
            None => {
                grid.label_at(left as f32, left, y, &text, dim, right, SMALL);
            }
        }
    }
    for (index, (cells, x, y)) in empty_buttons(area, measure).into_iter().enumerate() {
        if y >= area.bottom() {
            return;
        }
        let width = cells.chars().count() as u16;
        let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
        let label = if index == 0 { SETTINGS } else { START };
        // Both quiet buttons, lighter when pointed at: neither is a bare word.
        let fill = theme.quiet_button(if on_it { theme.row_hover } else { body.bg });
        let style = Style {
            bg: fill,
            fg: body.fg,
            ..body
        };
        ground(grid, x, y, width, body.bg);
        grid.button(x, y, &cells, label, fill, style);
    }
}

/// Nothing is painted under the pane, so a button's round corners need a ground to show on.
fn ground(grid: &mut Grid, x: u16, y: u16, width: u16, colour: Rgb) {
    let bounds = Bounds::cells(x as f32, y as f32, width as f32, 1.0);
    grid.panel(bounds, 0, Some(colour), None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str) -> Waiting {
        Waiting {
            kind: Kind::Finished,
            key: key.to_string(),
            title: format!("{key} finished"),
            detail: String::new(),
            since: Instant::now(),
            target: Target::Session(key.to_string()),
        }
    }

    #[test]
    fn news_about_the_same_thing_replaces_it_and_dealt_with_goes() {
        let mut inbox = Inbox::default();
        inbox.note(item("a"));
        inbox.note(item("b"));
        inbox.note(item("a"));
        assert_eq!(inbox.items.len(), 2);
        assert_eq!(inbox.items[1].key, "a", "newest last");
        assert_eq!(inbox.take_fresh().len(), 2);
        assert!(inbox.take_fresh().is_empty(), "told once");
        inbox.note(item("c"));
        inbox.drop_key("c");
        assert!(inbox.take_fresh().is_empty(), "gone before it was told");
        assert!(!inbox.has("c"));
    }

    #[test]
    fn one_notification_per_gap_and_a_burst_becomes_one() {
        let start = Instant::now();
        let mut notifier = Notifier::default();
        assert_eq!(notifier.due(start), None);
        notifier.push("api-server", "Claude Code asks to run sqlx migrate run");
        assert_eq!(
            notifier.due(start),
            Some((
                "Cellix · api-server".to_string(),
                "Claude Code asks to run sqlx migrate run".to_string()
            ))
        );
        notifier.push("infra", "Codex finished");
        notifier.push("paper", "Claude answered your comment");
        notifier.push("infra", "Tests failed");
        assert_eq!(
            notifier.due(start + Duration::from_secs(3)),
            None,
            "too soon"
        );
        assert_eq!(
            notifier.due(start + GAP),
            Some((
                "Cellix".to_string(),
                "3 things waiting in infra and paper".to_string()
            ))
        );
        assert_eq!(notifier.due(start + GAP * 3), None, "nothing left");
        notifier.push("infra", "Codex finished");
        notifier.clear();
        assert_eq!(notifier.due(start + GAP * 4), None, "the person came back");
    }
}
