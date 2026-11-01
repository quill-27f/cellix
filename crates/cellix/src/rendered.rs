//! Markdown as one editable page, the way Xcode's rendered editor works. The source stays
//! the document: this lays it out with its markers hidden, and every visible character
//! remembers its source position, so clicks and edits land in the right place.

use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Text,
    Heading,
    Code,
    Link,
    Dim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    tone: Tone,
    bold: bool,
    italic: bool,
    underline: bool,
}

const PLAIN: Look = Look {
    tone: Tone::Text,
    bold: false,
    italic: false,
    underline: false,
};

/// One visible character. `at` is its source position, and None marks decoration such as
/// a bullet or table padding, which has no source of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    pub at: Option<usize>,
    pub tone: Tone,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

fn glyph(ch: char, at: Option<usize>, look: Look) -> Glyph {
    Glyph {
        ch,
        at,
        tone: look.tone,
        bold: look.bold,
        italic: look.italic,
        underline: look.underline,
    }
}

fn decoration(text: &str, tone: Tone) -> Vec<Glyph> {
    let look = Look { tone, ..PLAIN };
    text.chars().map(|c| glyph(c, None, look)).collect()
}

/// One screen row of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub glyphs: Vec<Glyph>,
    /// Source line, counting from zero.
    pub line: usize,
    /// Where the caret goes at this row's end.
    pub end: usize,
    /// Whether the caret may stop here. Labels and table rules aren't text.
    pub stop: bool,
    /// The last row of its source line, when a line wraps.
    pub last: bool,
    /// Sits on the code background.
    pub shaded: bool,
    /// The column of a task checkbox, and the source position of its mark.
    pub checkbox: Option<(usize, usize)>,
    /// Where each table cell starts in the source.
    pub cells: Vec<usize>,
}

impl Row {
    fn caret_x(&self, pos: usize) -> usize {
        let mut x = 0;
        let mut before = None;
        for g in &self.glyphs {
            // The end of a table cell is a place of its own, before the padding.
            if g.at.is_some_and(|at| at >= pos) || (g.at.is_none() && before == Some(pos)) {
                return x;
            }
            before = g.at.map(|at| at + 1);
            x += g.ch.width().unwrap_or(0);
        }
        x
    }

    /// The source position at a column.
    pub fn position_at(&self, column: usize) -> usize {
        let mut x = 0;
        for (i, g) in self.glyphs.iter().enumerate() {
            x += g.ch.width().unwrap_or(0);
            if x > column {
                // Padding belongs to the text before it, as in a table cell.
                if g.at.is_none()
                    && let Some(before) = self.glyphs[..i].iter().rev().find_map(|g| g.at)
                {
                    return before + 1;
                }
                let next = self.glyphs[i..].iter().find_map(|g| g.at);
                return next.unwrap_or(self.end);
            }
        }
        self.end
    }
}

/// How a line starts, with its hidden marker.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Marker {
    Heading(usize),
    Quote,
    Bullet,
    Ordered(u64),
    Task { checked: bool, mark: usize },
}

struct Prefix {
    marker: Marker,
    indent: usize,
    /// Chars of source the marker takes up, including indent.
    hidden: usize,
}

fn prefix(line: &str) -> Option<Prefix> {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ').count();
    let rest = &chars[indent..];
    let hashes = rest.iter().take_while(|c| **c == '#').count();
    if (1..=6).contains(&hashes) && rest.get(hashes) == Some(&' ') {
        return Some(Prefix {
            marker: Marker::Heading(hashes),
            indent,
            hidden: indent + hashes + 1,
        });
    }
    if rest.len() >= 2 && rest[0] == '>' && rest[1] == ' ' {
        return Some(Prefix {
            marker: Marker::Quote,
            indent,
            hidden: indent + 2,
        });
    }
    if rest.len() >= 2 && matches!(rest[0], '-' | '*' | '+') && rest[1] == ' ' {
        if rest.len() >= 6
            && rest[2] == '['
            && matches!(rest[3], ' ' | 'x' | 'X')
            && rest[4] == ']'
            && rest[5] == ' '
        {
            return Some(Prefix {
                marker: Marker::Task {
                    checked: rest[3] != ' ',
                    mark: indent + 3,
                },
                indent,
                hidden: indent + 6,
            });
        }
        return Some(Prefix {
            marker: Marker::Bullet,
            indent,
            hidden: indent + 2,
        });
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && rest.get(digits) == Some(&'.') && rest.get(digits + 1) == Some(&' ') {
        let number: String = rest[..digits].iter().collect();
        return Some(Prefix {
            marker: Marker::Ordered(number.parse().unwrap_or(1)),
            indent,
            hidden: indent + digits + 2,
        });
    }
    None
}

fn is_rule(line: &str) -> bool {
    let text = line.trim();
    text.len() >= 3
        && (text.chars().all(|c| c == '-')
            || text.chars().all(|c| c == '*')
            || text.chars().all(|c| c == '_'))
}

/// Finds a closing marker in `chars` at or after `from`.
fn closing(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(marker.len() - 1))
        .find(|i| chars[*i..].starts_with(marker) && *i > from)
}

/// Turns one span of text into glyphs, hiding emphasis, code and link markers.
fn inline(chars: &[char], offset: usize, look: Look, out: &mut Vec<Glyph>) {
    let mut i = 0;
    while i < chars.len() {
        let rest = &chars[i..];
        let at = offset + i;
        let wrap = |marker: &[char], next: Look, out: &mut Vec<Glyph>| {
            let open = marker.len();
            let close = closing(rest, open, marker)?;
            inline(&rest[open..close], at + open, next, out);
            Some(close + open)
        };
        let bold = Look { bold: true, ..look };
        let italic = Look {
            italic: true,
            ..look
        };
        let used = if rest.starts_with(&['*', '*']) {
            wrap(&['*', '*'], bold, out)
        } else if rest.starts_with(&['_', '_']) {
            wrap(&['_', '_'], bold, out)
        } else if rest[0] == '*' && rest.get(1).is_some_and(|c| *c != ' ') {
            wrap(&['*'], italic, out)
        } else if rest[0] == '_'
            && rest.get(1).is_some_and(|c| *c != ' ')
            && (i == 0 || !chars[i - 1].is_alphanumeric())
        {
            wrap(&['_'], italic, out)
        } else if rest[0] == '`' {
            closing(rest, 1, &['`']).map(|close| {
                let code = Look {
                    tone: Tone::Code,
                    ..look
                };
                for (n, c) in rest[1..close].iter().enumerate() {
                    out.push(glyph(*c, Some(at + 1 + n), code));
                }
                close + 1
            })
        } else if rest[0] == '[' {
            link(rest, at, look, out)
        } else {
            None
        };
        match used {
            Some(count) => i += count,
            None => {
                out.push(glyph(chars[i], Some(at), look));
                i += 1;
            }
        }
    }
}

/// `[text](url)`: shows the text and hides the rest.
fn link(rest: &[char], at: usize, look: Look, out: &mut Vec<Glyph>) -> Option<usize> {
    let close = rest.iter().position(|c| *c == ']')?;
    if rest.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 1 + rest[close + 1..].iter().position(|c| *c == ')')?;
    let shown = Look {
        tone: Tone::Link,
        underline: true,
        ..look
    };
    inline(&rest[1..close], at + 1, shown, out);
    Some(end + 1)
}

/// Splits a line's glyphs into rows of at most `width`, breaking at spaces, and indents
/// continuation rows to line up under the text.
fn wrap(
    line: usize,
    end: usize,
    glyphs: Vec<Glyph>,
    lead: usize,
    width: usize,
    rows: &mut Vec<Row>,
) {
    let blank = Row {
        glyphs: Vec::new(),
        line,
        end,
        stop: true,
        last: true,
        shaded: false,
        checkbox: None,
        cells: Vec::new(),
    };
    let width = width.max(lead + 8);
    if glyphs.len() <= width {
        rows.push(Row { glyphs, ..blank });
        return;
    }
    let indent = decoration(&" ".repeat(lead), Tone::Text);
    let mut current: Vec<Glyph> = Vec::new();
    let mut rest = glyphs.into_iter().peekable();
    let mut pieces: Vec<Vec<Glyph>> = Vec::new();
    while let Some(g) = rest.next() {
        current.push(g);
        if current.len() >= width && rest.peek().is_some() {
            let cut = current
                .iter()
                .rposition(|g| g.ch == ' ')
                .filter(|i| *i > lead)
                .map_or(current.len(), |i| i + 1);
            let tail = current.split_off(cut);
            pieces.push(std::mem::replace(
                &mut current,
                indent.iter().copied().chain(tail).collect(),
            ));
        }
    }
    pieces.push(current);
    let count = pieces.len();
    let starts: Vec<usize> = pieces
        .iter()
        .map(|p| p.iter().find_map(|g| g.at).unwrap_or(end))
        .collect();
    for (i, glyphs) in pieces.into_iter().enumerate() {
        rows.push(Row {
            glyphs,
            end: starts.get(i + 1).copied().unwrap_or(end),
            last: i + 1 == count,
            ..blank.clone()
        });
    }
}

fn split_cells(line: &str) -> Vec<(usize, Vec<char>)> {
    let chars: Vec<char> = line.chars().collect();
    let mut cells = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i <= chars.len() {
        if i == chars.len() || chars[i] == '|' {
            if i > 0 || (i == 0 && chars.first() != Some(&'|')) {
                cells.push((start, chars[start..i].to_vec()));
            }
            start = i + 1;
        }
        i += 1;
    }
    if line.trim_end().ends_with('|') {
        cells.pop();
    }
    cells
        .into_iter()
        .map(|(start, cell)| {
            let lead = cell.iter().take_while(|c| **c == ' ').count();
            let tail = cell.iter().rev().take_while(|c| **c == ' ').count();
            let end = cell.len().saturating_sub(tail).max(lead);
            (start + lead, cell[lead..end].to_vec())
        })
        .collect()
}

fn is_table_rule(line: &str) -> bool {
    let text = line.trim();
    text.contains('-') && text.contains('|') && text.chars().all(|c| "|-: ".contains(c))
}

/// Lays out a table with its pipes and rule hidden and its columns aligned.
fn table(lines: &[(usize, &str)], first: usize, rows: &mut Vec<Row>) {
    let mut parsed: Vec<Vec<Vec<Glyph>>> = Vec::new();
    let mut starts: Vec<Vec<usize>> = Vec::new();
    for (index, (offset, line)) in lines.iter().enumerate() {
        if index == 1 {
            parsed.push(Vec::new());
            starts.push(Vec::new());
            continue;
        }
        let look = Look {
            bold: index == 0,
            ..PLAIN
        };
        let cells = split_cells(line);
        starts.push(cells.iter().map(|(at, _)| offset + at).collect());
        parsed.push(
            cells
                .iter()
                .map(|(at, cell)| {
                    let mut out = Vec::new();
                    inline(cell, offset + at, look, &mut out);
                    out
                })
                .collect(),
        );
    }
    let columns = parsed.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            parsed
                .iter()
                .filter_map(|r| r.get(c))
                .map(Vec::len)
                .max()
                .unwrap_or(0)
                .max(3)
        })
        .collect();
    let base = |line: usize, end: usize| Row {
        glyphs: Vec::new(),
        line,
        end,
        stop: true,
        last: true,
        shaded: false,
        checkbox: None,
        cells: Vec::new(),
    };
    for (index, cells) in parsed.iter().enumerate() {
        let (offset, text) = lines[index];
        let line = first + index;
        if index == 1 {
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            rows.push(Row {
                glyphs: decoration(&parts.join("─┼─"), Tone::Dim),
                stop: false,
                ..base(line, offset)
            });
            continue;
        }
        let mut glyphs = Vec::new();
        for (c, width) in widths.iter().enumerate() {
            if c > 0 {
                glyphs.extend(decoration(" │ ", Tone::Dim));
            }
            let cell = cells.get(c).cloned().unwrap_or_default();
            let used = cell.len();
            glyphs.extend(cell);
            let mut pad = decoration(&" ".repeat(width - used), Tone::Text);
            // An empty cell still needs somewhere for the caret to land.
            if used == 0
                && let (Some(first), Some(at)) = (pad.first_mut(), starts[index].get(c))
            {
                first.at = Some(*at);
            }
            glyphs.extend(pad);
        }
        let end = cells
            .iter()
            .flatten()
            .filter_map(|g| g.at)
            .max()
            .map_or(offset + text.chars().count(), |at| at + 1);
        rows.push(Row {
            glyphs,
            cells: starts[index].clone(),
            ..base(line, end)
        });
    }
}

fn shaded(line: usize, offset: usize, text: &str, tone: Tone) -> Row {
    let look = Look { tone, ..PLAIN };
    let glyphs: Vec<Glyph> = text
        .chars()
        .enumerate()
        .map(|(i, c)| glyph(c, Some(offset + i), look))
        .collect();
    Row {
        end: offset + text.chars().count(),
        glyphs,
        line,
        stop: true,
        last: true,
        shaded: true,
        checkbox: None,
        cells: Vec::new(),
    }
}

/// Lays the whole document out for a page `width` columns wide.
pub fn layout(text: &str, width: usize) -> Vec<Row> {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        lines.push((offset, line));
        offset += line.chars().count() + 1;
    }
    let mut rows = Vec::new();
    let mut i = 0;
    // Front matter shows as dimmed text, with its delimiters hidden.
    if lines[0].1 == "---"
        && let Some(close) = (1..lines.len()).find(|n| lines[*n].1 == "---")
    {
        for (n, (offset, line)) in lines.iter().enumerate().take(close).skip(1) {
            rows.push(shaded(n, *offset, line, Tone::Dim));
        }
        i = close + 1;
    }
    while i < lines.len() {
        let (offset, line) = lines[i];
        let trimmed = line.trim_start();
        if let Some(language) = trimmed.strip_prefix("```") {
            let close = (i + 1..lines.len())
                .find(|n| lines[*n].1.trim_start().starts_with("```"))
                .unwrap_or(lines.len());
            if !language.trim().is_empty() {
                rows.push(Row {
                    glyphs: decoration(language.trim(), Tone::Dim),
                    stop: false,
                    ..shaded(i, offset, "", Tone::Dim)
                });
            }
            for (n, (offset, line)) in lines.iter().enumerate().take(close).skip(i + 1) {
                rows.push(shaded(n, *offset, line, Tone::Code));
            }
            i = close + 1;
            continue;
        }
        if line.contains('|') && lines.get(i + 1).is_some_and(|n| is_table_rule(n.1)) {
            let end = (i + 2..lines.len())
                .find(|n| !lines[*n].1.contains('|'))
                .unwrap_or(lines.len());
            table(&lines[i..end], i, &mut rows);
            i = end;
            continue;
        }
        if is_rule(line) {
            rows.push(Row {
                glyphs: decoration(&"─".repeat(width.min(60)), Tone::Dim),
                line: i,
                end: offset,
                stop: true,
                last: true,
                shaded: false,
                checkbox: None,
                cells: Vec::new(),
            });
            i += 1;
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let found = prefix(line);
        let mut glyphs = Vec::new();
        let mut checkbox = None;
        let (hidden, look) = match &found {
            None => (0, PLAIN),
            Some(p) => {
                let depth = p.indent / 2;
                let pad = " ".repeat(p.indent);
                let look = match &p.marker {
                    Marker::Heading(level) => Look {
                        tone: Tone::Heading,
                        bold: true,
                        underline: *level == 1,
                        italic: false,
                    },
                    Marker::Quote => {
                        glyphs.extend(decoration("▌ ", Tone::Dim));
                        PLAIN
                    }
                    Marker::Bullet => {
                        let bullet = ["•", "◦", "▪"][depth % 3];
                        glyphs.extend(decoration(&format!("{pad}{bullet} "), Tone::Text));
                        PLAIN
                    }
                    Marker::Ordered(number) => {
                        glyphs.extend(decoration(&format!("{pad}{number}. "), Tone::Dim));
                        PLAIN
                    }
                    Marker::Task { checked, mark } => {
                        let symbol = if *checked { "☑" } else { "☐" };
                        checkbox = Some((p.indent, offset + mark));
                        glyphs.extend(decoration(&format!("{pad}{symbol} "), Tone::Link));
                        PLAIN
                    }
                };
                (p.hidden, look)
            }
        };
        let lead = glyphs.len();
        inline(&chars[hidden..], offset + hidden, look, &mut glyphs);
        let first = rows.len();
        wrap(i, offset + chars.len(), glyphs, lead, width, &mut rows);
        if let Some(row) = rows.get_mut(first) {
            row.checkbox = checkbox;
        }
        i += 1;
    }
    rows
}

/// The row and column of a source position. Positions inside hidden text land on the
/// nearest visible place.
pub fn locate(rows: &[Row], pos: usize) -> (usize, usize) {
    for (index, row) in rows.iter().enumerate() {
        let inside = if row.last {
            pos <= row.end
        } else {
            pos < row.end
        };
        if row.stop && inside {
            return (index, row.caret_x(pos));
        }
    }
    let last = rows.iter().rposition(|r| r.stop).unwrap_or(0);
    (last, rows.get(last).map_or(0, |r| r.caret_x(usize::MAX)))
}

/// Every place the caret can rest, in source order.
fn stops(rows: &[Row]) -> Vec<usize> {
    let mut all: Vec<usize> = rows
        .iter()
        .filter(|r| r.stop)
        .flat_map(|r| {
            let ends = r
                .glyphs
                .windows(2)
                .filter_map(|w| match (w[0].at, w[1].at) {
                    (Some(at), None) => Some(at + 1),
                    _ => None,
                });
            let starts = r.glyphs.iter().filter_map(|g| g.at);
            starts.chain(ends).chain([r.end]).collect::<Vec<_>>()
        })
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

pub fn snap(rows: &[Row], pos: usize) -> usize {
    let (row, x) = locate(rows, pos);
    rows.get(row).map_or(pos, |r| r.position_at(x))
}

pub fn step(rows: &[Row], pos: usize, forward: bool) -> usize {
    let all = stops(rows);
    let index = all.partition_point(|s| *s < pos);
    if forward {
        let next = if all.get(index) == Some(&pos) {
            index + 1
        } else {
            index
        };
        all.get(next).copied().unwrap_or(pos)
    } else {
        index.checked_sub(1).map_or(pos, |i| all[i])
    }
}

/// Moves `delta` caret rows, keeping to column `goal`.
pub fn vertical(rows: &[Row], pos: usize, delta: isize, goal: usize) -> usize {
    let (mut row, _) = locate(rows, pos);
    for _ in 0..delta.unsigned_abs() {
        let next = if delta < 0 {
            rows[..row].iter().rposition(|r| r.stop)
        } else {
            rows.iter()
                .skip(row + 1)
                .position(|r| r.stop)
                .map(|i| row + 1 + i)
        };
        match next {
            Some(found) => row = found,
            None => break,
        }
    }
    rows.get(row).map_or(pos, |r| r.position_at(goal))
}

pub fn row_edge(rows: &[Row], pos: usize, end: bool) -> usize {
    let (row, _) = locate(rows, pos);
    let Some(row) = rows.get(row) else {
        return pos;
    };
    if end {
        row.end
    } else {
        row.glyphs.iter().find_map(|g| g.at).unwrap_or(row.end)
    }
}

/// The next table cell after `pos`, for Tab.
pub fn next_cell(rows: &[Row], pos: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    let here = rows.get(row)?;
    if here.cells.is_empty() {
        return None;
    }
    let mut after = rows[row..]
        .iter()
        .take_while(|r| !r.cells.is_empty() || !r.stop)
        .flat_map(|r| r.cells.iter().copied());
    after.find(|at| *at > pos)
}

/// What Enter should do on a line, and Backspace at its start.
pub enum Intent {
    /// Start the next line with this marker.
    Continue(String),
    /// The item is empty: remove its marker, which ends the list.
    EndList(usize),
    Plain,
}

/// Enter at the end of a list item carries the list on.
pub fn enter_intent(line: &str) -> Intent {
    let Some(p) = prefix(line) else {
        return Intent::Plain;
    };
    if matches!(p.marker, Marker::Heading(_)) {
        return Intent::Plain;
    }
    if line.chars().count() == p.hidden {
        return Intent::EndList(p.hidden);
    }
    let pad = " ".repeat(p.indent);
    let chars: Vec<char> = line.chars().collect();
    Intent::Continue(match p.marker {
        Marker::Ordered(number) => format!("{pad}{}. ", number + 1),
        Marker::Task { .. } => format!("{pad}- [ ] "),
        _ => chars[..p.hidden].iter().collect(),
    })
}

/// Length of the hidden marker, which Backspace removes as a unit.
pub fn hidden_prefix(line: &str) -> usize {
    prefix(line).map_or(0, |p| p.hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(row: &Row) -> String {
        row.glyphs.iter().map(|g| g.ch).collect()
    }

    fn texts(source: &str) -> Vec<String> {
        layout(source, 40).iter().map(text).collect()
    }

    #[test]
    fn block_markers_are_hidden() {
        assert_eq!(texts("# Title"), ["Title"]);
        assert_eq!(texts("- item"), ["• item"]);
        assert_eq!(texts("  - inner"), ["  ◦ inner"]);
        assert_eq!(texts("3. third"), ["3. third"]);
        assert_eq!(texts("> said"), ["▌ said"]);
        assert_eq!(texts("- [ ] todo"), ["☐ todo"]);
        assert_eq!(texts("- [x] done"), ["☑ done"]);
    }

    #[test]
    fn inline_markers_are_hidden() {
        assert_eq!(texts("a **b** *c* `d` e"), ["a b c d e"]);
        assert_eq!(
            texts("see [the docs](http://x.y) now"),
            ["see the docs now"]
        );
    }

    #[test]
    fn unmatched_markers_stay_as_text() {
        assert_eq!(
            texts("2 * 3 and snake_case_name"),
            ["2 * 3 and snake_case_name"]
        );
        assert_eq!(texts("**open"), ["**open"]);
    }

    #[test]
    fn every_glyph_knows_its_source_position() {
        let source = "# Hi **yo**";
        let rows = layout(source, 40);
        let chars: Vec<char> = source.chars().collect();
        for g in &rows[0].glyphs {
            assert_eq!(chars[g.at.unwrap()], g.ch);
        }
    }

    #[test]
    fn fences_hide_and_the_language_is_a_label() {
        assert_eq!(
            texts("```rust\nlet x;\n```\nafter"),
            ["rust", "let x;", "after"]
        );
        let rows = layout("```rust\nlet x;\n```", 40);
        assert!(!rows[0].stop);
        assert!(rows[1].shaded);
    }

    #[test]
    fn tables_align_with_pipes_and_rule_hidden() {
        let rows = texts("| Path | Holds |\n|---|---|\n| a | long text |");
        assert_eq!(rows[0], "Path │ Holds    ");
        assert_eq!(rows[1], "─────┼──────────");
        assert_eq!(rows[2], "a    │ long text");
    }

    #[test]
    fn front_matter_delimiters_hide() {
        assert_eq!(texts("---\ntitle: x\n---\nbody"), ["title: x", "body"]);
    }

    #[test]
    fn long_lines_wrap_under_their_text() {
        let rows = layout("- one two three four", 12);
        assert_eq!(text(&rows[0]), "• one two ");
        assert_eq!(text(&rows[1]), "  three four");
        assert!(!rows[0].last);
        assert!(rows[1].last);
    }

    #[test]
    fn the_caret_skips_hidden_markers() {
        let rows = layout("# Hi\nnext", 40);
        assert_eq!(snap(&rows, 0), 2);
        assert_eq!(step(&rows, 2, false), 2);
        assert_eq!(step(&rows, 4, true), 5);
        assert_eq!(locate(&rows, 2), (0, 0));
    }

    #[test]
    fn vertical_moves_keep_the_column_and_skip_rules() {
        let rows = layout("| a |\n|---|\n| b |", 40);
        let down = vertical(&rows, 2, 1, 0);
        assert_eq!(rows[locate(&rows, down).0].line, 2);
    }

    #[test]
    fn tab_finds_the_next_cell() {
        let source = "| a | b |\n|---|---|\n| c | d |";
        let rows = layout(source, 40);
        let chars: Vec<char> = source.chars().collect();
        let first = next_cell(&rows, 2).unwrap();
        assert_eq!(chars[first], 'b');
        let second = next_cell(&rows, first).unwrap();
        assert_eq!(chars[second], 'c');
    }

    #[test]
    fn enter_carries_lists_on_and_empty_items_end_them() {
        assert!(matches!(enter_intent("- a"), Intent::Continue(s) if s == "- "));
        assert!(matches!(enter_intent("  2. a"), Intent::Continue(s) if s == "  3. "));
        assert!(matches!(enter_intent("- [x] a"), Intent::Continue(s) if s == "- [ ] "));
        assert!(matches!(enter_intent("> a"), Intent::Continue(s) if s == "> "));
        assert!(matches!(enter_intent("- "), Intent::EndList(2)));
        assert!(matches!(enter_intent("# a"), Intent::Plain));
        assert!(matches!(enter_intent("plain"), Intent::Plain));
    }
}
