/// Most matches kept, so a one-letter search in a huge file stays quick.
const MAX_MATCHES: usize = 10_000;

use cellix_core::pattern::{Options, char_ranges};

/// A screen Find looks through that isn't a file or a terminal: everything it lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Looked {
    /// The list in front of the side pane.
    #[default]
    Pane,
    /// The changes Review shows.
    Review,
    /// The Task page: its timeline and the step beside it.
    Task,
}

/// Find in the current file, and replace when that row is open. Matches are char ranges.
#[derive(Default)]
pub struct Find {
    pub query: String,
    pub matches: Vec<(usize, usize)>,
    pub current: usize,
    pub options: Options,
    /// What matches become, while the replace row shows.
    pub replace: Option<String>,
    /// The replace field has the keyboard.
    pub in_replace: bool,
    /// Why the query can't be searched for, as a regular expression that doesn't parse.
    pub invalid: Option<String>,
    /// It looks through the terminal tab in front of the side pane, not a file. Matches are
    /// char ranges of all its lines joined, the scrollback's first.
    pub pane: bool,
    /// It looks through the list in front of the side pane: comments, the Inbox, problems,
    /// agents or a build's log. Matches are char ranges of `rows` joined.
    pub listed: bool,
    /// Every row that list holds, as it draws them from its top, not only those in view.
    pub rows: Vec<String>,
    /// Which screen `listed` looks through.
    pub looked: Looked,
    /// How many of `rows` each part of that screen has: a part scrolls on its own, as the
    /// Task page's two columns do.
    pub parts: Vec<usize>,
}

impl Find {
    /// Searches again, keeping to the first match at or after `from`.
    pub fn search(&mut self, text: &str, from: usize) {
        self.matches.clear();
        self.invalid = None;
        match self.options.matcher(&self.query) {
            Ok(matcher) => self.matches = char_ranges(&matcher, text, MAX_MATCHES),
            Err(why) if !self.query.is_empty() => self.invalid = Some(why),
            Err(_) => {}
        }
        self.current = self.matches.iter().position(|m| m.0 >= from).unwrap_or(0);
    }

    /// The field that has the keyboard.
    pub fn field_mut(&mut self) -> &mut String {
        match &mut self.replace {
            Some(with) if self.in_replace => with,
            _ => &mut self.query,
        }
    }

    pub fn step(&mut self, delta: isize) {
        let count = self.matches.len() as isize;
        if count > 0 {
            self.current = (self.current as isize + delta).rem_euclid(count) as usize;
        }
    }

    /// Each match in a list's rows, in order, as (the part it's in, where it starts in its
    /// row, its letters): what tells which of them a match on screen is.
    pub fn listed_matches(&self) -> Vec<(usize, usize, String)> {
        let mut starts = Vec::with_capacity(self.rows.len());
        let mut at = 0;
        for row in &self.rows {
            starts.push(at);
            at += row.chars().count() + 1;
        }
        self.matches
            .iter()
            .map(|&(start, end)| {
                let row = starts.partition_point(|s| *s <= start).saturating_sub(1);
                let from = start - starts.get(row).copied().unwrap_or(0);
                let letters = self.rows.get(row).map_or_else(String::new, |r| {
                    r.chars().skip(from).take(end - start).collect()
                });
                (self.part_of(row), from, letters)
            })
            .collect()
    }

    /// The row of its own part that match `index` starts in.
    pub fn row_in_part(&self, index: usize) -> usize {
        let Some(&(start, _)) = self.matches.get(index) else {
            return 0;
        };
        let mut at = 0;
        let mut row = 0;
        for (n, text) in self.rows.iter().enumerate() {
            let next = at + text.chars().count() + 1;
            if start < next {
                row = n;
                break;
            }
            at = next;
        }
        let part = self.part_of(row);
        row - self.parts.iter().take(part).sum::<usize>()
    }

    /// The part of the screen that row of `rows` is in.
    fn part_of(&self, row: usize) -> usize {
        let mut before = 0;
        for (part, rows) in self.parts.iter().enumerate() {
            before += rows;
            if row < before {
                return part;
            }
        }
        self.parts.len().saturating_sub(1)
    }

    pub fn selected(&self) -> Option<(usize, usize)> {
        self.matches.get(self.current).copied()
    }

    pub fn count_label(&self) -> String {
        if self.invalid.is_some() {
            "Invalid".to_string()
        } else if self.matches.is_empty() {
            "No results".to_string()
        } else {
            format!("{} of {}", self.current + 1, self.matches.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(query: &str, text: &str, from: usize) -> Find {
        let mut find = Find {
            query: query.to_string(),
            ..Find::default()
        };
        find.search(text, from);
        find
    }

    #[test]
    fn matches_ignore_case_and_count_in_chars() {
        let found = find("cafÉ", "é café CAFÉ", 0);
        assert_eq!(found.matches, vec![(2, 6), (7, 11)]);
    }

    #[test]
    fn stepping_wraps_around() {
        let mut found = find("a", "a a a", 3);
        assert_eq!(found.current, 2);
        found.step(1);
        assert_eq!(found.current, 0);
        found.step(-1);
        assert_eq!(found.count_label(), "3 of 3");
    }

    #[test]
    fn nothing_found_says_so() {
        assert_eq!(find("z", "a", 0).count_label(), "No results");
        assert_eq!(find("", "a", 0).selected(), None);
    }
}
