//! The commit graph in Source Control: a row per commit, its lanes drawn as straight lines
//! with square joins, which both the cell grid and the paint layer can draw.

use cellix_render::{Bounds, Grid, Measure, Rect, Rgb, Style};
use cellix_review::{Change, Commit, GraphRow};

use crate::theme::Theme;

/// Columns each lane takes.
const LANE: u16 = 2;
/// Lanes past this many are drawn in the last one, so the message keeps its room.
pub const MOST_LANES: usize = 4;
/// Columns a message keeps before a branch's name may stand beside it.
const MESSAGE_ROOM: usize = 14;
/// Where a lane's line stands within its two columns.
const LINE_AT: f32 = 0.5;

/// The commits listed, their lanes, and the one opened to show its files.
#[derive(Default)]
pub struct Graph {
    pub commits: Vec<Commit>,
    pub rows: Vec<GraphRow>,
    /// The commit opened to its files, by its hash, with them.
    pub open: Option<(String, Vec<Change>)>,
}

impl Graph {
    pub fn new(commits: Vec<Commit>) -> Self {
        let rows = cellix_review::graph(&commits);
        Self {
            commits,
            rows,
            open: None,
        }
    }

    /// How many lane columns the rows use, at most `MOST_LANES`.
    pub fn lanes(&self) -> usize {
        let widest = self.rows.iter().map(GraphRow::width).max().unwrap_or(1);
        widest.clamp(1, MOST_LANES)
    }

    /// Columns from a row's left edge to its message.
    pub fn text_from(&self) -> u16 {
        self.lanes() as u16 * LANE + 1
    }

    pub fn is_open(&self, index: usize) -> bool {
        let hash = self.commits.get(index).map(|c| c.hash.as_str());
        self.open.as_ref().map(|(h, _)| h.as_str()) == hash
    }
}

fn lane_colour(lane: usize, theme: &Theme) -> Rgb {
    let colours = [
        theme.accent,
        theme.git_modified,
        theme.git_added,
        theme.attention,
    ];
    colours[lane % colours.len()]
}

/// Draws a commit's lanes at `x`, one row tall. Cells get box-drawing characters; the pixel
/// tiers get hairlines and a dot over them.
pub fn draw_lanes(
    grid: &mut Grid,
    x: u16,
    y: u16,
    row: &GraphRow,
    merge: bool,
    theme: &Theme,
    behind: Style,
) {
    let clamp = |lane: usize| lane.min(MOST_LANES - 1);
    let column = |lane: usize| x + clamp(lane) as u16 * LANE;
    let centre = |lane: usize| column(lane) as f32 + LINE_AT;
    let width = row.width();
    let cell = |grid: &mut Grid, at: u16, symbol: &str, colour: Rgb| {
        let style = Style {
            fg: colour,
            painted: true,
            ..behind
        };
        grid.put_str(at, y, symbol, style, at + 1);
    };
    for lane in 0..width {
        let above = row.above.get(lane).copied().unwrap_or(false);
        let below = row.below.get(lane).copied().unwrap_or(false);
        if lane == row.lane || !(above || below) {
            continue;
        }
        let colour = lane_colour(lane, theme);
        let joins = row.joins.contains(&lane);
        let forks = row.forks.contains(&lane);
        let symbol = match (joins || forks, lane > row.lane) {
            (false, _) if above && below => "│",
            (false, _) if above => "╵",
            (false, _) => "╷",
            (true, true) if above && !below => "╯",
            (true, true) if below && !above => "╮",
            (true, false) if above && !below => "╰",
            (true, false) if below && !above => "╭",
            (true, _) => "┼",
        };
        cell(grid, column(lane), symbol, colour);
        let top = if above { 0.0 } else { 0.5 };
        let bottom = if below { 1.0 } else { 0.5 };
        let line = Bounds::cells(centre(lane), y as f32 + top, 0.0, bottom - top);
        grid.hairline(line, colour);
    }
    if let Some((low, high)) = row.across() {
        let from = column(low) + 1;
        let to = column(high);
        let colour = lane_colour(if high > row.lane { high } else { low }, theme);
        for at in from..to {
            if at != column(row.lane) {
                cell(grid, at, "─", colour);
            }
        }
        let line = Bounds::cells(centre(low), y as f32 + 0.5, centre(high) - centre(low), 0.0);
        grid.hairline(line, colour);
    }
    // The commit's own lane, and its dot over the lines.
    let lane = row.lane;
    let colour = lane_colour(lane, theme);
    let above = row.above.get(lane).copied().unwrap_or(false);
    let below = row.below.get(lane).copied().unwrap_or(false);
    if above || below {
        let top = if above { 0.0 } else { 0.5 };
        let bottom = if below { 1.0 } else { 0.5 };
        let line = Bounds::cells(centre(lane), y as f32 + top, 0.0, bottom - top);
        grid.hairline(line, colour);
    }
    cell(grid, column(lane), if merge { "○" } else { "●" }, colour);
    // A cell is about twice as tall as it's wide, so this is near round; the corners are
    // half its height, a share of a row, which is what a panel's radius counts in.
    let dot = Bounds::cells(centre(lane) - 0.32, y as f32 + 0.35, 0.64, 0.3);
    if merge {
        grid.panel(dot, 15, Some(behind.bg), Some(colour));
    } else {
        grid.panel(dot, 15, Some(colour), None);
    }
}

/// Lanes running on past a row that isn't a commit's, as an open commit's files.
pub fn draw_through(
    grid: &mut Grid,
    (x, y): (u16, u16),
    lanes: &[bool],
    theme: &Theme,
    behind: Style,
) {
    for (lane, _) in lanes.iter().enumerate().filter(|(_, on)| **on) {
        let at = x + lane.min(MOST_LANES - 1) as u16 * LANE;
        let colour = lane_colour(lane, theme);
        let style = Style {
            fg: colour,
            painted: true,
            ..behind
        };
        grid.put_str(at, y, "│", style, at + 1);
        let line = Bounds::cells(at as f32 + LINE_AT, y as f32, 0.0, 1.0);
        grid.hairline(line, colour);
    }
}

/// A commit's row: lanes, the branches at it, its message and its age.
pub fn draw_commit(
    grid: &mut Grid,
    graph: &Graph,
    (index, row, now): (usize, Rect, i64),
    measure: &dyn Measure,
    theme: &Theme,
    behind: Style,
) {
    let (Some(commit), Some(lanes)) = (graph.commits.get(index), graph.rows.get(index)) else {
        return;
    };
    draw_lanes(grid, row.x, row.y, lanes, commit.is_merge(), theme, behind);
    let quiet = Style {
        fg: theme.gutter.fg,
        ..behind
    };
    let age = cellix_review::age(commit.time, now);
    let right = row.right();
    grid.label_right(right, row.y, &age, quiet, 90);
    let end = right.saturating_sub(age.chars().count() as u16 + 1);
    let mut x = row.x + graph.text_from();
    // A branch's name only where the message keeps room to be read.
    let room = end.saturating_sub(x) as usize;
    let fits = |name: &String| room >= name.chars().count() + 1 + MESSAGE_ROOM;
    if let Some(name) = commit.refs.first().filter(|n| fits(n)) {
        let mark = Style {
            fg: theme.accent,
            ..behind
        };
        x = grid.label(x, row.y, name, mark, end) + 1;
    }
    let room = end.saturating_sub(x + 1) as f32;
    let summary = crate::source::clip(&commit.summary, room, measure);
    grid.label(x, row.y, &summary, behind, end.saturating_sub(1));
}
