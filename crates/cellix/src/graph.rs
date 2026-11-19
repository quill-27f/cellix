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

/// A commit's row: lanes, the branches and tags at it, its message and its age. `ground` is
/// what the row stands on in paint, a band's colour where it's pointed at or open.
pub fn draw_commit(
    grid: &mut Grid,
    graph: &Graph,
    (index, row, now): (usize, Rect, i64),
    measure: &dyn Measure,
    theme: &Theme,
    (behind, ground): (Style, Rgb),
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
    // The cells the message may take end two before the age's; in paint it ends a column
    // before the age's measured words, where the cells' count left 72 px empty after
    // "Bound the queue" and cut the message for nothing.
    let cells_end = right.saturating_sub(age.chars().count() as u16 + 2);
    let stop = if measure.in_cells() {
        f32::from(cells_end)
    } else {
        let small = cellix_render::Font {
            size: 90,
            ..cellix_render::Font::default()
        };
        f32::from(right) - measure.width(&age, small) - 1.0
    };
    let start = f32::from(row.x + graph.text_from());
    let chips = ref_chips(&commit.refs, (start, stop), measure);
    let colour = lane_colour(lanes.lane, theme);
    let mut x = start;
    for (name, at, wide) in &chips {
        draw_ref(
            grid,
            (*at, *wide, row.y),
            name,
            (colour, ground),
            behind,
            measure,
        );
        x = at + wide + ref_gap(measure);
    }
    let summary = crate::source::clip(&commit.summary, stop - x, measure);
    let painted = Style {
        painted: true,
        ..behind
    };
    grid.put_str(x.ceil() as u16, row.y, &summary, painted, cells_end);
    let font = cellix_render::Font::default();
    grid.text_run(
        (x, f32::from(row.y)),
        stop,
        &summary,
        (behind.fg, false),
        font,
    );
}

/// A branch's or tag's chip before a commit's message, as VS Code's graph has them: its name
/// in the colour of the commit's lane on a wash of it, in a chip's words (nine tenths of the
/// interface's) and at a chip's height, which the rasterizer stands at 0.714 of a taller row.
const REF_SIZE: u8 = 90;
const REF_HIGH: f32 = 0.72;
/// How strong the lane's colour is in a chip's wash: the "needs you" chip's.
const REF_WASH: f32 = 0.16;
/// Columns either side of a chip's words, 10 px of a 17 px column, as the status bar's chips
/// have; and between two chips, or the last and the message, 8.5 px.
const REF_PAD: f32 = 0.6;
const REF_GAP: f32 = 0.5;
/// The fewest letters a chip cut short keeps.
const REF_LETTERS: usize = 3;

fn ref_font() -> cellix_render::Font {
    cellix_render::Font {
        size: REF_SIZE,
        ..cellix_render::Font::default()
    }
}

/// Columns between two chips, and between the last and the message: a space on cells.
fn ref_gap(measure: &dyn Measure) -> f32 {
    if measure.in_cells() { 1.0 } else { REF_GAP }
}

/// The chips before a commit's message, as the name each shows, the column it starts at and
/// how many it takes, laid from `from` toward `end`, where the message has to end. They take
/// what the message leaves past `MESSAGE_ROOM`, and at least half of it, so in a narrow
/// sidebar both are cut rather than the names dropped. A name too long for its room is cut in
/// its middle, so both ends of "agent/queue-bound" show; one that can't keep three letters
/// isn't shown, nor any after it. On cells a chip is its name on the wash, a space before what
/// follows.
pub fn ref_chips(
    refs: &[String],
    (from, end): (f32, f32),
    measure: &dyn Measure,
) -> Vec<(String, f32, f32)> {
    let pad = if measure.in_cells() { 0.0 } else { REF_PAD };
    let gap = ref_gap(measure);
    let font = ref_font();
    let room = (end - from).max(0.0);
    let budget = (room - MESSAGE_ROOM as f32).max(room / 2.0);
    let mut chips = Vec::new();
    let mut at = from;
    for name in refs {
        let words = from + budget - at - gap - pad * 2.0;
        let shown = crate::source::clip_middle_as(name, words, font, measure);
        let wide = measure.width(&shown, font);
        let letters = shown.chars().filter(|c| *c != '…').count();
        if wide > words || letters < REF_LETTERS.min(name.chars().count()) {
            break;
        }
        // Its ink in its middle: the measure runs on past the last letter.
        let chip = wide - crate::measures::words_run_on(measure) + pad * 2.0;
        chips.push((shown, at, chip));
        at += chip + gap;
    }
    chips
}

/// One chip, `wide` columns from `x` on row `y`, in `colour` washed over `ground`: on a band
/// it's washed over the band, which where a commit is open is the blue over the sidebar, so
/// main's chip was the band's own colour and went. On cells, its name in the lane's colour on
/// the wash, the cells it takes.
fn draw_ref(
    grid: &mut Grid,
    (x, wide, y): (f32, f32, u16),
    name: &str,
    (colour, ground): (Rgb, Rgb),
    behind: Style,
    measure: &dyn Measure,
) {
    let cells = measure.in_cells();
    let under = if cells { behind.bg } else { ground };
    let wash = crate::theme::mix(under, colour, REF_WASH);
    let style = Style {
        fg: colour,
        // In paint the row keeps its own ground and only the chip is washed.
        bg: if cells { wash } else { behind.bg },
        painted: true,
        ..behind
    };
    let (cell, end) = (x.floor() as u16, (x + wide).ceil() as u16);
    grid.put_str(cell, y, name, style, end);
    if cells {
        return;
    }
    let top = f32::from(y) + (1.0 - REF_HIGH) / 2.0;
    grid.panel(Bounds::cells(x, top, wide, REF_HIGH), 22, Some(wash), None);
    // Its words by their font's span in the chip's middle, as a button's are, and their ink,
    // not their measure, in the middle across: centred by the measure, it had 10 px before it
    // and 13 after.
    let middle = x + (wide + crate::measures::words_run_on(measure)) / 2.0;
    grid.button_label(middle, f32::from(y), name, colour, REF_SIZE);
}
