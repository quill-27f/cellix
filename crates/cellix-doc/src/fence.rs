//! Fenced code blocks as CommonMark reads them: a block opened by three or more backticks or
//! tildes is closed only by a run of the same character at least as long, with nothing after
//! it. So a fence of four can hold a line of three, as a transcript's tool output does.

/// An open fence: its character and how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    mark: char,
    length: usize,
}

impl Fence {
    /// The fence `line` opens, with its info string (the language), if it opens one.
    pub fn open(line: &str) -> Option<(Fence, &str)> {
        let trimmed = line.trim_start();
        let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let length = trimmed.chars().take_while(|c| *c == mark).count();
        let info = &trimmed[length..];
        // A backtick fence's info can't hold a backtick, or it's inline code.
        if length < 3 || (mark == '`' && info.contains('`')) {
            return None;
        }
        Some((Fence { mark, length }, info.trim()))
    }

    /// Whether `line` closes this fence.
    pub fn closes(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.chars().count() >= self.length && trimmed.chars().all(|c| c == self.mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_run_as_long_of_the_same_mark_closes() {
        let (fence, info) = Fence::open("````rust").unwrap();
        assert_eq!(info, "rust");
        assert!(!fence.closes("```"));
        assert!(!fence.closes("~~~~"));
        assert!(!fence.closes("```` and more"));
        assert!(fence.closes("````"));
        assert!(fence.closes("  `````  "));
        let (tilde, _) = Fence::open("~~~").unwrap();
        assert!(!tilde.closes("```"));
        assert!(tilde.closes("~~~"));
        assert_eq!(Fence::open("``"), None);
        assert_eq!(Fence::open("``` a ` b"), None);
        assert_eq!(Fence::open("text"), None);
    }
}
