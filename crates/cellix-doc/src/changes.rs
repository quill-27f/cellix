//! Suggested edits kept in the text itself, so they survive any tool and travel with the file.
//! Markdown uses CriticMarkup and a paper uses the `changes` package.

/// How a document writes its changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Critic,
    Latex,
}

impl Syntax {
    /// What opens and closes an insertion.
    pub fn insertion(self) -> (&'static str, &'static str) {
        match self {
            Self::Critic => ("{++", "++}"),
            Self::Latex => ("\\added{", "}"),
        }
    }

    /// What opens and closes a deletion.
    pub fn deletion(self) -> (&'static str, &'static str) {
        match self {
            Self::Critic => ("{--", "--}"),
            Self::Latex => ("\\deleted{", "}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Insert,
    Delete,
    Replace,
}

/// One change, as a range of chars in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub old: String,
    pub new: String,
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars()
        .enumerate()
        .all(|(index, wanted)| chars.get(at + index) == Some(&wanted))
}

fn find_text(chars: &[char], from: usize, text: &str) -> Option<usize> {
    (from..chars.len()).find(|at| starts(chars, *at, text))
}

fn string(chars: &[char]) -> String {
    chars.iter().collect()
}

fn critic(chars: &[char]) -> Vec<Change> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 2 < chars.len() {
        let change = if starts(chars, at, "{++") {
            find_text(chars, at + 3, "++}").map(|close| Change {
                start: at,
                end: close + 3,
                kind: Kind::Insert,
                old: String::new(),
                new: string(&chars[at + 3..close]),
            })
        } else if starts(chars, at, "{--") {
            find_text(chars, at + 3, "--}").map(|close| Change {
                start: at,
                end: close + 3,
                kind: Kind::Delete,
                old: string(&chars[at + 3..close]),
                new: String::new(),
            })
        } else if starts(chars, at, "{~~") {
            find_text(chars, at + 3, "~>").and_then(|arrow| {
                let close = find_text(chars, arrow, "~~}")?;
                Some(Change {
                    start: at,
                    end: close + 3,
                    kind: Kind::Replace,
                    old: string(&chars[at + 3..arrow]),
                    new: string(&chars[arrow + 2..close]),
                })
            })
        } else {
            None
        };
        match change {
            Some(change) => {
                at = change.end;
                found.push(change);
            }
            None => at += 1,
        }
    }
    found
}

/// Where the brace opened at `open` closes.
fn brace_end(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// A brace argument after a command, stepping over any `[options]`.
fn argument(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut open = from;
    if chars.get(open) == Some(&'[') {
        open += chars[open..].iter().position(|c| *c == ']')? + 1;
    }
    (chars.get(open) == Some(&'{')).then_some(())?;
    Some((open, brace_end(chars, open)?))
}

fn latex(chars: &[char]) -> Vec<Change> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let change = if starts(chars, at, "\\added") {
            argument(chars, at + 6).map(|(open, close)| Change {
                start: at,
                end: close + 1,
                kind: Kind::Insert,
                old: String::new(),
                new: string(&chars[open + 1..close]),
            })
        } else if starts(chars, at, "\\deleted") {
            argument(chars, at + 8).map(|(open, close)| Change {
                start: at,
                end: close + 1,
                kind: Kind::Delete,
                old: string(&chars[open + 1..close]),
                new: String::new(),
            })
        } else if starts(chars, at, "\\replaced") {
            argument(chars, at + 9).and_then(|(open, close)| {
                let (second, end) = argument(chars, close + 1)?;
                Some(Change {
                    start: at,
                    end: end + 1,
                    kind: Kind::Replace,
                    old: string(&chars[second + 1..end]),
                    new: string(&chars[open + 1..close]),
                })
            })
        } else {
            None
        };
        match change {
            Some(change) => {
                at = change.end;
                found.push(change);
            }
            None => at += 1,
        }
    }
    found
}

pub fn find(text: &str, syntax: Syntax) -> Vec<Change> {
    let chars: Vec<char> = text.chars().collect();
    match syntax {
        Syntax::Critic => critic(&chars),
        Syntax::Latex => latex(&chars),
    }
}

/// The document with every change kept or dropped.
/// The change from `old` to `new` as suggestions in `old`, word by word, so whoever reviews it
/// accepts or rejects each one. Accepting all of them gives `new` back.
pub fn suggest(old: &str, new: &str, syntax: Syntax) -> String {
    let diff = similar::TextDiff::from_words(old, new);
    let mut out = String::with_capacity(new.len() + new.len() / 4);
    let (mut gone, mut added) = (String::new(), String::new());
    let flush = |out: &mut String, gone: &mut String, added: &mut String| {
        // Space at a change's edges stays plain, so the marks sit on words.
        let lead = common_lead(gone, added).len();
        let trail = common_trail(&gone[lead..], &added[lead..]).len();
        out.push_str(&gone[..lead]);
        let (old, new) = (
            &gone[lead..gone.len() - trail],
            &added[lead..added.len() - trail],
        );
        out.push_str(&mark(old, new, syntax));
        out.push_str(&gone[gone.len() - trail..]);
        gone.clear();
        added.clear();
    };
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Equal => {
                if !gone.is_empty() || !added.is_empty() {
                    flush(&mut out, &mut gone, &mut added);
                }
                out.push_str(change.value());
            }
            similar::ChangeTag::Delete => gone.push_str(change.value()),
            similar::ChangeTag::Insert => added.push_str(change.value()),
        }
    }
    if !gone.is_empty() || !added.is_empty() {
        flush(&mut out, &mut gone, &mut added);
    }
    out
}

/// One suggestion in the document's own syntax.
fn mark(old: &str, new: &str, syntax: Syntax) -> String {
    match (syntax, old.is_empty(), new.is_empty()) {
        (_, true, true) => String::new(),
        (Syntax::Critic, true, false) => format!("{{++{new}++}}"),
        (Syntax::Critic, false, true) => format!("{{--{old}--}}"),
        (Syntax::Critic, false, false) => format!("{{~~{old}~>{new}~~}}"),
        (Syntax::Latex, true, false) => format!("\\added{{{new}}}"),
        (Syntax::Latex, false, true) => format!("\\deleted{{{old}}}"),
        (Syntax::Latex, false, false) => format!("\\replaced{{{new}}}{{{old}}}"),
    }
}

/// The whitespace both start with.
fn common_lead<'a>(a: &'a str, b: &str) -> &'a str {
    let n = a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y && x.is_whitespace())
        .map(|(x, _)| x.len_utf8())
        .sum();
    &a[..n]
}

/// The whitespace both end with.
fn common_trail<'a>(a: &'a str, b: &str) -> &'a str {
    let n: usize = a
        .chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y && x.is_whitespace())
        .map(|(x, _)| x.len_utf8())
        .sum();
    &a[a.len() - n..]
}

pub fn settle_all(text: &str, syntax: Syntax, accept: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for change in find(text, syntax) {
        out.extend(&chars[from..change.start]);
        out.push_str(if accept { &change.new } else { &change.old });
        from = change.end;
    }
    out.extend(&chars[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_edit_becomes_suggestions_that_accept_back_to_it() {
        let old = "The cache goes first, then the index.\n";
        let new = "The cache goes last, then the new index.\n";
        let marked = suggest(old, new, Syntax::Critic);
        assert_eq!(
            marked,
            "The cache goes {~~first,~>last,~~} then the {++new ++}index.\n"
        );
        assert_eq!(settle_all(&marked, Syntax::Critic, true), new);
        assert_eq!(settle_all(&marked, Syntax::Critic, false), old);
        let tex = suggest("A b c.\n", "A x c.\n", Syntax::Latex);
        assert_eq!(tex, "A \\replaced{x}{b} c.\n");
    }

    #[test]
    fn markdown_writes_its_changes_in_three_ways() {
        let text = "a {++new++} b {--old--} c {~~was~>now~~} d";
        let found = find(text, Syntax::Critic);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].kind, Kind::Insert);
        assert_eq!(found[0].new, "new");
        assert_eq!(found[1].old, "old");
        assert_eq!(
            (found[2].old.as_str(), found[2].new.as_str()),
            ("was", "now")
        );
        assert_eq!(settle_all(text, Syntax::Critic, true), "a new b  c now d");
        assert_eq!(settle_all(text, Syntax::Critic, false), "a  b old c was d");
    }

    #[test]
    fn a_paper_uses_the_changes_package() {
        let text = "x \\added{new} y \\deleted[id=a]{old} z \\replaced{now}{was {nested}}";
        let found = find(text, Syntax::Latex);
        assert_eq!(found.len(), 3);
        assert_eq!(found[1].old, "old");
        assert_eq!(found[2].old, "was {nested}");
        assert_eq!(settle_all(text, Syntax::Latex, true), "x new y  z now");
        assert_eq!(
            settle_all(text, Syntax::Latex, false),
            "x  y old z was {nested}"
        );
    }

    #[test]
    fn places_count_chars_not_bytes() {
        let text = "é {++ü++}";
        let change = &find(text, Syntax::Critic)[0];
        assert_eq!((change.start, change.end), (2, 9));
    }

    #[test]
    fn an_unclosed_mark_is_just_text() {
        assert!(find("a {++b", Syntax::Critic).is_empty());
        assert_eq!(settle_all("a {++b", Syntax::Critic, true), "a {++b");
    }
}
