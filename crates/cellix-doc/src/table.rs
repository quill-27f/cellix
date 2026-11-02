//! Editing a Markdown table as a table. Every change rewrites it with its columns lined up,
//! so the source stays as readable as the page.

/// What can be done to a table, from the cell the caret is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    RowAbove,
    RowBelow,
    ColumnLeft,
    ColumnRight,
    DeleteRow,
    DeleteColumn,
    Align(Align),
    Delete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Default,
    Left,
    Centre,
    Right,
}

/// The rows, the alignment of each column, without the rule between header and body.
#[derive(Debug, PartialEq, Eq)]
struct Table {
    rows: Vec<Vec<String>>,
    aligns: Vec<Align>,
}

/// Splits a row at its pipes, leaving `\|` and pipes in code alone.
fn cells(line: &str) -> Vec<String> {
    let text = line.trim();
    let text = text.strip_prefix('|').unwrap_or(text);
    let text = text.strip_suffix('|').unwrap_or(text);
    let mut out = vec![String::new()];
    let (mut escaped, mut code) = (false, false);
    for c in text.chars() {
        if c == '|' && !escaped && !code {
            out.push(String::new());
        } else if let Some(cell) = out.last_mut() {
            cell.push(c);
        }
        code ^= c == '`';
        escaped = c == '\\' && !escaped;
    }
    out.iter().map(|c| c.trim().to_string()).collect()
}

fn align_of(cell: &str) -> Align {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Centre,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        _ => Align::Default,
    }
}

fn parse(text: &str) -> Option<Table> {
    let lines: Vec<&str> = text.lines().collect();
    let rule = lines.get(1)?;
    let aligns: Vec<Align> = cells(rule).iter().map(|c| align_of(c)).collect();
    let mut rows: Vec<Vec<String>> = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, line)| cells(line))
        .collect();
    let columns = rows
        .iter()
        .map(Vec::len)
        .chain([aligns.len()])
        .max()
        .unwrap_or(1);
    for row in &mut rows {
        row.resize(columns, String::new());
    }
    let mut aligns = aligns;
    aligns.resize(columns, Align::Default);
    Some(Table { rows, aligns })
}

fn print(table: &Table) -> String {
    let columns = table.aligns.len();
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            let longest = table.rows.iter().map(|r| r[c].chars().count());
            longest.max().unwrap_or(0).max(3)
        })
        .collect();
    let line = |row: &[String]| {
        let shown: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        format!("| {} |", shown.join(" | "))
    };
    let rule: Vec<String> = table
        .aligns
        .iter()
        .zip(&widths)
        .map(|(align, width)| {
            let dashes = width.saturating_sub(match align {
                Align::Centre => 2,
                Align::Default => 0,
                _ => 1,
            });
            let dashes = "-".repeat(dashes);
            match align {
                Align::Default => dashes,
                Align::Left => format!(":{dashes}"),
                Align::Right => format!("{dashes}:"),
                Align::Centre => format!(":{dashes}:"),
            }
        })
        .collect();
    let mut out = vec![line(&table.rows[0]), format!("| {} |", rule.join(" | "))];
    out.extend(table.rows[1..].iter().map(|row| line(row)));
    out.join("\n")
}

/// Where a cell's text starts in printed table text, in chars.
pub fn cell_offset(text: &str, row: usize, column: usize) -> usize {
    let line = if row == 0 { 0 } else { row + 1 };
    let mut at = 0;
    for (index, source) in text.split('\n').enumerate() {
        if index == line {
            let mut pipes = 0;
            for (i, c) in source.chars().enumerate() {
                if c == '|' {
                    pipes += 1;
                    if pipes == column + 1 {
                        return at + i + 2;
                    }
                }
            }
        }
        at += source.chars().count() + 1;
    }
    0
}

/// Applies an edit to a table's text, from the cell at `(row, column)`, counting the header
/// as row zero and leaving out the rule. Returns the new text, and the cell to land in.
/// Deleting the table, or its only row or column, returns None for the text.
pub fn apply(text: &str, row: usize, column: usize, edit: Edit) -> Option<(String, usize, usize)> {
    let mut table = parse(text)?;
    let columns = table.aligns.len();
    let blank = || vec![String::new(); columns];
    let (mut row, mut column) = (row.min(table.rows.len() - 1), column.min(columns - 1));
    match edit {
        // A table always has its header, so a row above that goes under it.
        Edit::RowAbove if row == 0 => {
            table.rows.insert(1, blank());
            row = 1;
        }
        Edit::RowAbove => table.rows.insert(row, blank()),
        Edit::RowBelow => {
            table.rows.insert(row + 1, blank());
            row += 1;
        }
        Edit::ColumnLeft | Edit::ColumnRight => {
            let at = column + usize::from(edit == Edit::ColumnRight);
            for cells in &mut table.rows {
                cells.insert(at, String::new());
            }
            table.aligns.insert(at, Align::Default);
            column = at;
        }
        Edit::DeleteRow if table.rows.len() == 1 => return None,
        Edit::DeleteRow if row == 0 => {
            // The first body row becomes the header.
            table.rows.remove(0);
        }
        Edit::DeleteRow => {
            table.rows.remove(row);
            row = row.min(table.rows.len() - 1);
        }
        Edit::DeleteColumn if columns == 1 => return None,
        Edit::DeleteColumn => {
            for cells in &mut table.rows {
                cells.remove(column);
            }
            table.aligns.remove(column);
            column = column.min(columns - 2);
        }
        Edit::Align(align) => table.aligns[column] = align,
        Edit::Delete => return None,
    }
    Some((print(&table), row, column))
}

/// A new, empty table with a header row.
pub fn new_table(rows: usize, columns: usize) -> String {
    let rows = rows.max(2);
    let columns = columns.max(1);
    let table = Table {
        rows: (0..rows)
            .map(|row| {
                (0..columns)
                    .map(|c| {
                        if row == 0 {
                            format!("Column {}", c + 1)
                        } else {
                            String::new()
                        }
                    })
                    .collect()
            })
            .collect(),
        aligns: vec![Align::Default; columns],
    };
    print(&table)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";

    fn rows(text: &str) -> Vec<Vec<String>> {
        parse(text).unwrap().rows
    }

    #[test]
    fn rows_go_in_above_and_below() {
        let (text, row, _) = apply(TABLE, 1, 0, Edit::RowBelow).unwrap();
        assert_eq!(rows(&text).len(), 4);
        assert_eq!(rows(&text)[2], ["", ""]);
        assert_eq!(row, 2);
        let (text, row, _) = apply(TABLE, 2, 0, Edit::RowAbove).unwrap();
        assert_eq!(rows(&text)[2], ["", ""]);
        assert_eq!(rows(&text)[3], ["3", "4"]);
        assert_eq!(row, 2);
    }

    #[test]
    fn a_row_above_the_header_goes_under_it() {
        let (text, row, _) = apply(TABLE, 0, 0, Edit::RowAbove).unwrap();
        assert_eq!(rows(&text)[0], ["A", "B"]);
        assert_eq!(rows(&text)[1], ["", ""]);
        assert_eq!(row, 1);
    }

    #[test]
    fn columns_go_in_either_side() {
        let (text, _, column) = apply(TABLE, 1, 0, Edit::ColumnRight).unwrap();
        assert_eq!(rows(&text)[1], ["1", "", "2"]);
        assert_eq!(column, 1);
        let (text, _, column) = apply(TABLE, 1, 1, Edit::ColumnLeft).unwrap();
        assert_eq!(rows(&text)[0], ["A", "", "B"]);
        assert_eq!(column, 1);
    }

    #[test]
    fn rows_and_columns_come_out() {
        let (text, ..) = apply(TABLE, 1, 0, Edit::DeleteRow).unwrap();
        assert_eq!(rows(&text), [["A", "B"], ["3", "4"]]);
        let (text, ..) = apply(TABLE, 1, 0, Edit::DeleteColumn).unwrap();
        assert_eq!(rows(&text), [["B"], ["2"], ["4"]]);
        let (text, ..) = apply(TABLE, 0, 0, Edit::DeleteRow).unwrap();
        assert_eq!(
            rows(&text)[0],
            ["1", "2"],
            "the next row becomes the header"
        );
    }

    #[test]
    fn the_last_of_anything_takes_the_table_with_it() {
        assert_eq!(apply("| A |\n|---|", 0, 0, Edit::DeleteColumn), None);
        assert_eq!(apply("| A |\n|---|", 0, 0, Edit::DeleteRow), None);
        assert_eq!(apply(TABLE, 1, 1, Edit::Delete), None);
    }

    #[test]
    fn alignment_is_written_in_the_rule() {
        let (text, ..) = apply(TABLE, 1, 1, Edit::Align(Align::Right)).unwrap();
        assert!(text.lines().nth(1).unwrap().contains("--:"), "{text}");
        let (again, ..) = apply(&text, 1, 0, Edit::Align(Align::Centre)).unwrap();
        assert!(again.lines().nth(1).unwrap().contains(":-:"), "{again}");
        let parsed = parse(&again).unwrap();
        assert_eq!(parsed.aligns, [Align::Centre, Align::Right]);
    }

    #[test]
    fn pipes_in_code_and_escapes_are_text() {
        let table = "| a |\n|---|\n| `x|y` |\n| p\\|q |";
        assert_eq!(rows(table)[1], ["`x|y`"]);
        assert_eq!(rows(table)[2], ["p\\|q"]);
    }

    #[test]
    fn the_caret_can_find_a_cell_again() {
        let (text, row, column) = apply(TABLE, 2, 0, Edit::ColumnRight).unwrap();
        let at = cell_offset(&text, row, column);
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(chars[at - 2], '|');
        assert_eq!(chars[at - 1], ' ');
        let first = cell_offset(&text, 0, 0);
        assert_eq!(chars[first], 'A');
    }

    #[test]
    fn a_new_table_has_a_header_and_its_size() {
        let text = new_table(3, 4);
        assert_eq!(rows(&text).len(), 3);
        assert_eq!(rows(&text)[0][3], "Column 4");
        assert_eq!(text.lines().count(), 4);
    }
}
