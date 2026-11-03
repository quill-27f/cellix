/// Most matches kept, so a one-letter search in a huge file stays quick.
const MAX_MATCHES: usize = 10_000;

use cellix_core::pattern::{Options, char_ranges};

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
