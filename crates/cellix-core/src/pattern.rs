//! What a search looks for: text or a regular expression, matching case or not, whole words
//! or anywhere. Find in a file and Search in the project share it.

use regex::{NoExpand, Regex, RegexBuilder};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Upper and lower case differ.
    pub case: bool,
    /// Only where a word starts and ends.
    pub whole: bool,
    /// The query is a regular expression.
    pub regex: bool,
}

/// The three switches, as their buttons show them, in order.
pub const SWITCHES: [&str; 3] = ["Aa", "ab", ".*"];
/// What each does, with its key.
pub const SWITCH_NAMES: [&str; 3] = [
    "Match Case (Alt+C)",
    "Match Whole Word (Alt+W)",
    "Use Regular Expression (Alt+R)",
];

impl Options {
    pub fn is_on(&self, switch: usize) -> bool {
        [self.case, self.whole, self.regex][switch.min(2)]
    }

    pub fn flip(&mut self, switch: usize) {
        match switch {
            0 => self.case = !self.case,
            1 => self.whole = !self.whole,
            _ => self.regex = !self.regex,
        }
    }

    /// Alt+C, Alt+W and Alt+R, as in VS Code. Returns whether the key was one of them.
    pub fn flip_key(&mut self, key: char) -> bool {
        let switch = match key.to_ascii_lowercase() {
            'c' => 0,
            'w' => 1,
            'r' => 2,
            _ => return false,
        };
        self.flip(switch);
        true
    }

    /// The query as something to match with, or why it can't be one.
    pub fn matcher(&self, query: &str) -> Result<Regex, String> {
        if query.is_empty() {
            return Err("Nothing to find".to_string());
        }
        let body = if self.regex {
            query.to_string()
        } else {
            regex::escape(query)
        };
        let body = if self.whole {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        RegexBuilder::new(&body)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|_| "Not a valid regular expression".to_string())
    }

    /// `text` with every match replaced. With a regular expression `$1` takes a group.
    pub fn replace_all(&self, matcher: &Regex, text: &str, with: &str) -> String {
        if self.regex {
            matcher.replace_all(text, with).into_owned()
        } else {
            matcher.replace_all(text, NoExpand(with)).into_owned()
        }
    }

    /// What one match becomes.
    pub fn replacement(&self, matcher: &Regex, found: &str, with: &str) -> String {
        if self.regex {
            matcher.replace(found, with).into_owned()
        } else {
            with.to_string()
        }
    }
}

/// Where it matches, as char ranges, at most `limit` of them. Empty matches don't count.
pub fn char_ranges(matcher: &Regex, text: &str, limit: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let (mut chars, mut last) = (0, 0);
    for hit in matcher.find_iter(text) {
        if hit.start() == hit.end() {
            continue;
        }
        chars += text[last..hit.start()].chars().count();
        let length = hit.as_str().chars().count();
        found.push((chars, chars + length));
        chars += length;
        last = hit.end();
        if found.len() >= limit {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(query: &str, options: Options, text: &str) -> Vec<(usize, usize)> {
        char_ranges(&options.matcher(query).unwrap(), text, 100)
    }

    #[test]
    fn case_whole_words_and_expressions_narrow_a_search() {
        let text = "Cache cache cached é cache";
        let plain = Options::default();
        assert_eq!(ranges("cache", plain, text).len(), 4, "any case, anywhere");
        let case = Options {
            case: true,
            ..plain
        };
        assert_eq!(ranges("cache", case, text).len(), 3);
        let whole = Options {
            whole: true,
            ..plain
        };
        assert_eq!(ranges("cache", whole, text).len(), 3, "not cached");
        assert_eq!(
            ranges("cache", whole, text)[2],
            (21, 26),
            "counted in chars"
        );
        let regex = Options {
            regex: true,
            ..plain
        };
        assert_eq!(ranges("cach(e|ed)", regex, text).len(), 4);
        assert!(regex.matcher("(").is_err());
        assert_eq!(
            ranges("a.b", plain, "a.b axb").len(),
            1,
            "plain text is literal"
        );
    }

    #[test]
    fn a_replacement_takes_groups_only_from_an_expression() {
        let regex = Options {
            regex: true,
            ..Options::default()
        };
        let matcher = regex.matcher(r"(\w+)@(\w+)").unwrap();
        assert_eq!(regex.replace_all(&matcher, "a@b c@d", "$2.$1"), "b.a d.c");
        let plain = Options::default();
        let matcher = plain.matcher("x").unwrap();
        assert_eq!(plain.replace_all(&matcher, "x X", "$1"), "$1 $1");
    }
}
