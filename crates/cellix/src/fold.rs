//! Where code can fold: from the language server when it says, and otherwise from the
//! indentation, as VS Code does for a language with no folding of its own.

use cellix_core::Rope;

use crate::rows::width_of;

/// A region that folds: line `start` stays, as its header, and `start + 1..=end` hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

/// A line that is only closing brackets, such as `}` or `});`, which folds in with the block
/// it closes and shows after the fold's chip.
pub fn is_closer(text: &str) -> bool {
    let text = text.trim();
    text.starts_with(['}', ')', ']']) && text.chars().all(|c| "}])>;,".contains(c))
}

fn line_text(rope: &Rope, line: usize) -> String {
    rope.line(line)
        .to_string()
        .trim_end_matches(['\n', '\r'])
        .to_string()
}

/// A line's indent in columns, or `None` when it's blank.
fn indent(text: &str, tab: usize) -> Option<usize> {
    let mut column = 0;
    for c in text.chars() {
        match c {
            ' ' | '\t' => column += width_of(c, column, tab),
            _ => return Some(column),
        }
    }
    None
}

/// Each line more indented than the one before it opens a region, which runs to the last
/// line indented further, with a closing bracket at the header's own indent folded in.
pub fn by_indent(rope: &Rope, tab: usize) -> Vec<Range> {
    let lines: Vec<String> = (0..rope.len_lines()).map(|l| line_text(rope, l)).collect();
    let indents: Vec<Option<usize>> = lines.iter().map(|l| indent(l, tab)).collect();
    let mut found = Vec::new();
    // Open regions, as (header line, its indent).
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut previous: Option<(usize, usize)> = None;
    for (line, depth) in indents.iter().enumerate() {
        let Some(depth) = *depth else {
            continue;
        };
        if let Some((before, before_depth)) = previous {
            if depth > before_depth {
                open.push((before, before_depth));
            }
            while let Some(&(header, header_depth)) = open.last() {
                if header_depth < depth {
                    break;
                }
                open.pop();
                let closes = header_depth == depth && is_closer(&lines[line]);
                let end = if closes { line } else { before };
                found.push(Range { start: header, end });
            }
        }
        previous = Some((line, depth));
    }
    if let Some((last, _)) = previous {
        for (header, _) in open {
            found.push(Range {
                start: header,
                end: last,
            });
        }
    }
    found.retain(|r| r.end > r.start);
    found.sort();
    found
}

/// The server's regions, as (first line, last line), made to fold in their closing bracket
/// as the indent's do, whichever line the server ends them on.
pub fn from_server(rope: &Rope, ranges: &[(usize, usize)]) -> Vec<Range> {
    let lines = rope.len_lines();
    let mut found: Vec<Range> = ranges
        .iter()
        .filter(|(start, end)| end > start && *end < lines)
        .map(|&(start, end)| {
            let next = end + 1;
            let closes = next < lines && !is_closer(&line_text(rope, end)) && {
                let text = line_text(rope, next);
                is_closer(&text) && indent(&text, 4) == indent(&line_text(rope, start), 4)
            };
            Range {
                start,
                end: if closes { next } else { end },
            }
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

/// What a fold shows after its chip: the closing bracket it took in, if it took one.
pub fn closer(rope: &Rope, range: Range) -> Option<String> {
    let text = line_text(rope, range.end);
    is_closer(&text).then(|| text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str =
        "impl A {\n    fn a() {\n        x();\n\n        y();\n    }\n\n    fn b() {}\n}\nlast\n";

    #[test]
    fn indents_open_regions_and_closers_fold_in() {
        let found = by_indent(&Rope::from_str(CODE), 4);
        assert_eq!(
            found,
            vec![Range { start: 0, end: 8 }, Range { start: 1, end: 5 }],
            "the blank line inside `a` doesn't end it"
        );
        let python = "def f():\n    a\n    b\nc\n";
        assert_eq!(
            by_indent(&Rope::from_str(python), 4),
            vec![Range { start: 0, end: 2 }]
        );
        // `} else {` opens the next block, so it isn't folded into the one before.
        let branches = "if a {\n    b\n} else {\n    c\n}\n";
        assert_eq!(
            by_indent(&Rope::from_str(branches), 4),
            vec![Range { start: 0, end: 1 }, Range { start: 2, end: 4 }]
        );
    }

    #[test]
    fn a_servers_regions_take_in_their_closer() {
        let rope = Rope::from_str(CODE);
        assert_eq!(
            from_server(&rope, &[(1, 4), (0, 8), (3, 3)]),
            vec![Range { start: 0, end: 8 }, Range { start: 1, end: 5 }]
        );
        assert_eq!(
            closer(&rope, Range { start: 1, end: 5 }).as_deref(),
            Some("}")
        );
        assert_eq!(closer(&rope, Range { start: 1, end: 4 }), None);
    }
}
