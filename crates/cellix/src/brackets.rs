//! Bracket pairs in code: each bracket's depth, for its colour, and its partner, for the
//! caret's match. Brackets inside strings and comments don't count.

use cellix_core::Rope;

/// Past this, finding pairs on every edit costs more than the colours are worth.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bracket {
    /// Its char position.
    pub at: usize,
    /// How many pairs it's inside: 0 for one at the top.
    pub depth: usize,
    /// The char position of the bracket that closes or opens it.
    pub partner: Option<usize>,
}

fn opener_of(c: char) -> Option<char> {
    match c {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// Every bracket in `rope`, in order. `skip` holds sorted, non-overlapping char ranges of
/// strings and comments.
pub fn find(rope: &Rope, skip: &[(usize, usize)]) -> Vec<Bracket> {
    let mut found: Vec<Bracket> = Vec::new();
    // Open brackets, as (char, index into `found`).
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut skipping = skip.iter().peekable();
    for (at, c) in rope.chars().enumerate() {
        if !matches!(c, '(' | ')' | '[' | ']' | '{' | '}') {
            continue;
        }
        while skipping.peek().is_some_and(|(_, end)| *end <= at) {
            skipping.next();
        }
        if skipping.peek().is_some_and(|(start, _)| *start <= at) {
            continue;
        }
        match opener_of(c) {
            None => {
                open.push((c, found.len()));
                found.push(Bracket {
                    at,
                    depth: open.len() - 1,
                    partner: None,
                });
            }
            // A closer that doesn't close the bracket last opened is left alone, so one
            // stray bracket doesn't throw off every pair after it.
            Some(opener) if open.last().is_some_and(|(o, _)| *o == opener) => {
                let (_, index) = open.pop().expect("checked");
                found[index].partner = Some(at);
                found.push(Bracket {
                    at,
                    depth: open.len(),
                    partner: Some(found[index].at),
                });
            }
            Some(_) => found.push(Bracket {
                at,
                depth: open.len(),
                partner: None,
            }),
        }
    }
    found
}

/// The pair the caret touches: a bracket just after it, or else just before it.
pub fn pair_at(brackets: &[Bracket], head: usize) -> Option<(usize, usize)> {
    let at = |position: usize| {
        let index = brackets.binary_search_by_key(&position, |b| b.at).ok()?;
        let bracket = brackets[index];
        bracket
            .partner
            .map(|p| (bracket.at.min(p), bracket.at.max(p)))
    };
    at(head).or_else(|| head.checked_sub(1).and_then(at))
}

/// The brackets from char `from` up to `to`.
pub fn between(brackets: &[Bracket], from: usize, to: usize) -> &[Bracket] {
    let start = brackets.partition_point(|b| b.at < from);
    let end = brackets.partition_point(|b| b.at < to);
    &brackets[start..end.max(start)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depths(text: &str, skip: &[(usize, usize)]) -> Vec<(char, usize, bool)> {
        let rope = Rope::from_str(text);
        find(&rope, skip)
            .iter()
            .map(|b| (rope.char(b.at), b.depth, b.partner.is_some()))
            .collect()
    }

    #[test]
    fn pairs_take_their_depth_and_find_each_other() {
        assert_eq!(
            depths("f(a[1], {b})", &[]),
            vec![
                ('(', 0, true),
                ('[', 1, true),
                (']', 1, true),
                ('{', 1, true),
                ('}', 1, true),
                (')', 0, true),
            ]
        );
        let rope = Rope::from_str("f(a[1])");
        let found = find(&rope, &[]);
        assert_eq!(pair_at(&found, 1), Some((1, 6)), "before the opener");
        assert_eq!(pair_at(&found, 7), Some((1, 6)), "after the closer");
        assert_eq!(pair_at(&found, 4), Some((3, 5)), "after the inner opener");
    }

    #[test]
    fn strings_and_comments_and_strays_are_left_out() {
        // `"(" ` is a string from 2 to 5.
        let found = depths("f(\"(\")", &[(2, 5)]);
        assert_eq!(found, vec![('(', 0, true), (')', 0, true)]);
        let stray = depths("{ ) }", &[]);
        assert_eq!(stray, vec![('{', 0, true), (')', 1, false), ('}', 0, true)]);
        let unclosed = depths("{ (", &[]);
        assert_eq!(unclosed, vec![('{', 0, false), ('(', 1, false)]);
    }
}
