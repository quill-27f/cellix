//! A table of data as a page: real columns, with every cell editable in place. A long file
//! shows its first rows, since nobody reads the rest row by row.

use super::*;

/// Rows set out on the page. Past this a file stays usable, and its source has the rest.
const MAX_ROWS: usize = 2000;
/// The narrowest a column is pushed to when they don't all fit.
const NARROW: usize = 8;
/// Space a column's edges take up.
const EDGES: usize = 2;

/// A row's fields, each with where its text starts. Quotes are hidden, not parsed away, so
/// every char still has its place in the source.
fn fields(line: &str, separator: char) -> Vec<(usize, Vec<char>)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    loop {
        let quoted = chars.get(i) == Some(&'"');
        if quoted {
            let body = i + 1;
            let mut end = body;
            while end < chars.len() {
                if chars[end] == '"' {
                    if chars.get(end + 1) == Some(&'"') {
                        end += 2;
                        continue;
                    }
                    break;
                }
                end += 1;
            }
            out.push((body, chars[body..end.min(chars.len())].to_vec()));
            i = end + 1;
            while i < chars.len() && chars[i] != separator {
                i += 1;
            }
        } else {
            while i < chars.len() && chars[i] != separator {
                i += 1;
            }
            out.push((start, chars[start..i.min(chars.len())].to_vec()));
        }
        if i >= chars.len() {
            return out;
        }
        i += 1;
        start = i;
    }
}

fn is_number(cell: &[char]) -> bool {
    let text: String = cell.iter().filter(|c| **c != ',').collect();
    let text = text.trim();
    !text.is_empty() && text.parse::<f64>().is_ok()
}

pub(super) fn layout(
    lines: &[(usize, &str)],
    separator: char,
    width: f32,
    context: &Context,
    rows: &mut Vec<Row>,
) {
    let present: Vec<usize> = (0..lines.len())
        .filter(|i| !lines[*i].1.is_empty())
        .collect();
    let shown = &present[..present.len().min(MAX_ROWS)];
    let table: Vec<Vec<(usize, Vec<char>)>> = shown
        .iter()
        .map(|i| fields(lines[*i].1, separator))
        .collect();
    let count = table.iter().map(Vec::len).max().unwrap_or(0).max(1);
    // Every column gets what it needs if they all fit, or else the longest give way.
    let longest: Vec<usize> = (0..count)
        .map(|c| {
            let cells = table.iter().filter_map(|row| row.get(c));
            cells.map(|(_, cell)| cell.len()).max().unwrap_or(0)
        })
        .collect();
    let needed: usize = longest.iter().map(|w| w + EDGES).sum();
    let fair = (width as usize / count).saturating_sub(EDGES).max(NARROW);
    let fits = needed <= width as usize;
    let body = table.iter().skip(1);
    let aligns: Vec<char> = (0..count)
        .map(|c| {
            let mut cells = body.clone().filter_map(|row| row.get(c)).peekable();
            let numbers = cells.peek().is_some() && cells.all(|(_, cell)| is_number(cell));
            if numbers { 'r' } else { 'l' }
        })
        .collect();
    let mut parsed = Vec::with_capacity(table.len());
    let mut starts = Vec::with_capacity(table.len());
    let mut places = Vec::with_capacity(table.len());
    for (row, cells) in table.iter().enumerate() {
        let (offset, line) = lines[shown[row]];
        let look = Look {
            font: Font {
                bold: row == 0,
                ..Font::default()
            },
            ..PLAIN
        };
        let glyphs = cells.iter().map(|(at, cell)| {
            let long = !fits && cell.len() > fair;
            let kept = if long { fair - 1 } else { cell.len() };
            let mut out: Vec<Glyph> = cell[..kept]
                .iter()
                .enumerate()
                .map(|(n, c)| glyph(*c, Some(offset + at + n), look))
                .collect();
            if long {
                out.extend(decoration("…", Tone::Dim));
            }
            measure_glyphs(&mut out, context.measure);
            out
        });
        parsed.push(glyphs.collect());
        starts.push(cells.iter().map(|(at, _)| offset + at).collect());
        places.push((shown[row], offset + line.chars().count()));
    }
    place_table(parsed, starts, &places, &aligns, rows);
    if present.len() > MAX_ROWS {
        let more = present.len();
        let note =
            format!("Showing the first {MAX_ROWS} of {more} rows. Show Source for the rest.");
        let last = lines.len() - 1;
        rows.push(note_row(&note, last, lines[last].0, context));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &str, separator: char) -> Vec<(usize, String)> {
        let found = fields(line, separator);
        found
            .into_iter()
            .map(|(at, cell)| (at, cell.into_iter().collect()))
            .collect()
    }

    #[test]
    fn fields_keep_their_places() {
        assert_eq!(
            plain("a,bb,,c", ','),
            [
                (0, "a".to_string()),
                (2, "bb".to_string()),
                (5, String::new()),
                (6, "c".to_string())
            ]
        );
        assert_eq!(plain("a\tb", '\t')[1], (2, "b".to_string()));
    }

    #[test]
    fn quotes_are_hidden_and_commas_inside_them_stay() {
        assert_eq!(
            plain("\"one, two\",3", ','),
            [(1, "one, two".to_string()), (11, "3".to_string())]
        );
    }
}
