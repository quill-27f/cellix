//! What a paper needs that isn't on the page: which file to build, what to build it with,
//! what went wrong and where, and Markdown turned into LaTeX for export.

use crate::diagram::{Kind, fenced_tikz, is_tikz, kind_of};
use crate::figure;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::table::{Align, Edit};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A problem from a build, at a line counting from one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

/// The file a build starts from: the one a `% !TEX root` comment names, this one if it's a
/// whole document, or a whole document nearby that takes this one in.
pub fn root_of(path: &Path, text: &str) -> PathBuf {
    let folder = path.parent().unwrap_or(Path::new(""));
    let named = text.lines().take(5).find_map(|line| {
        let rest = line.trim().strip_prefix('%')?.trim();
        let rest = rest.strip_prefix("!TEX")?.trim().strip_prefix("root")?;
        Some(rest.trim_start_matches([' ', '=']).trim().to_string())
    });
    if let Some(name) = named {
        return folder.join(name);
    }
    if text.contains("\\documentclass") {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let stem = stem.unwrap_or_default();
    for place in [folder, folder.parent().unwrap_or(folder)] {
        let Ok(entries) = std::fs::read_dir(place) else {
            continue;
        };
        for candidate in entries.flatten().map(|e| e.path()) {
            if candidate.extension().is_none_or(|e| e != "tex") || candidate == path {
                continue;
            }
            let Ok(other) = std::fs::read_to_string(&candidate) else {
                continue;
            };
            if other.contains("\\documentclass") && other.contains(&stem) {
                return candidate;
            }
        }
    }
    path.to_path_buf()
}

/// A command a paper defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Macro {
    pub name: String,
    pub arguments: usize,
    pub body: String,
}

/// Where numbering stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub algorithms: usize,
    /// Chapter, section, subsection and subsubsection.
    pub sections: [usize; 4],
    pub equations: usize,
    pub figures: usize,
    pub tables: usize,
    /// By counter name, for theorem-like environments.
    pub theorems: HashMap<String, usize>,
    /// After `\appendix`, sections count in letters.
    pub appendix: bool,
}

impl Counters {
    /// Counts a heading, and returns the number it prints with. Level 0 is a chapter.
    pub fn heading(&mut self, level: usize) -> String {
        self.sections[level] += 1;
        self.sections[level + 1..].fill(0);
        let first = usize::from(self.sections[0] == 0);
        let parts: Vec<String> = (first..=level)
            .map(|n| {
                let value = self.sections[n];
                if self.appendix && n == first {
                    char::from(b'A' + (value.saturating_sub(1) % 26) as u8).to_string()
                } else {
                    value.to_string()
                }
            })
            .collect();
        parts.join(".")
    }
}

/// A theorem-like environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theorem {
    pub title: String,
    /// The counter it shares, usually its own.
    pub counter: String,
    pub numbered: bool,
}

/// Names that need no `\newtheorem` to be read as what they are.
const THEOREMS: [(&str, &str); 12] = [
    ("theorem", "Theorem"),
    ("thm", "Theorem"),
    ("lemma", "Lemma"),
    ("lem", "Lemma"),
    ("proposition", "Proposition"),
    ("prop", "Proposition"),
    ("corollary", "Corollary"),
    ("cor", "Corollary"),
    ("definition", "Definition"),
    ("defn", "Definition"),
    ("remark", "Remark"),
    ("example", "Example"),
];

/// A paper as LaTeX sees it: what's defined and how things number, across all its files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub macros: HashMap<String, Macro>,
    pub theorems: HashMap<String, Theorem>,
    /// What each label will print as.
    pub labels: HashMap<String, String>,
    /// Numbering as each file starts.
    pub starts: HashMap<PathBuf, Counters>,
    /// Numbering as each file ends, so the file that `\\input`s it numbers on from there.
    pub after: HashMap<PathBuf, Counters>,
    /// What each citation key shows as: author and year.
    pub citations: HashMap<String, String>,
    /// The paper's own packages and commands, which a piece built alone still needs.
    pub setup: Vec<String>,
    /// Each `.bib` entry as its reference reads: authors, year and title.
    pub references: HashMap<String, String>,
    /// The `\\bibliographystyle`, which says how references are numbered.
    pub style: String,
    /// The keys the paper cites, in the order it first cites them.
    pub cited: Vec<String>,
    /// The `.bib` files the paper names with `\\bibliography` or `\\addbibresource`.
    pub bibliographies: Vec<PathBuf>,
}

/// The argument of `\name{...}` on a line, as a byte range.
fn braced(text: &str, open: usize) -> Option<(usize, usize)> {
    if !text[open..].starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + i));
                }
            }
            _ => {}
        }
    }
    None
}

fn word_after<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    let start = line.find(command)? + command.len();
    let (from, to) = braced(line, start)?;
    Some(&line[from..to])
}

/// Each row of an `align`, as the number it prints with. Starred or `\notag` rows have none,
/// and `\tag` names its own.
pub fn equation_rows(source: &str, numbered: bool, equations: &mut usize) -> Vec<Option<String>> {
    display_rows(source)
        .iter()
        .map(|row| {
            if let Some(tag) = word_after(row, "\\tag") {
                return Some(tag.to_string());
            }
            if !numbered || row.contains("\\notag") || row.contains("\\nonumber") {
                return None;
            }
            *equations += 1;
            Some(equations.to_string())
        })
        .collect()
}

/// A display's rows: one for an `equation`, one for each `\\` of an `align`.
pub fn display_rows(source: &str) -> Vec<&str> {
    if !source.contains("{align") && !source.contains("{gather") {
        return vec![source];
    }
    let bare = |row: &&str| {
        let row = row.trim();
        !row.is_empty() && !(row.starts_with("\\end{") && row.ends_with('}'))
    };
    source.split("\\\\").filter(bare).collect()
}

fn read_macro(line: &str, project: &mut Project) {
    let text = line.trim_start();
    for command in ["\\newcommand", "\\renewcommand", "\\providecommand"] {
        let Some(rest) = text.strip_prefix(command) else {
            continue;
        };
        let rest = rest.strip_prefix('*').unwrap_or(rest);
        let (name, after) = if let Some(inner) = rest.strip_prefix('{') {
            let Some((name, after)) = inner.split_once('}') else {
                return;
            };
            (name.to_string(), after)
        } else {
            let end = rest[1..]
                .find(|c: char| !c.is_ascii_alphabetic())
                .map_or(rest.len(), |i| i + 1);
            (rest[..end].to_string(), &rest[end..])
        };
        let mut after = after;
        let mut arguments = 0;
        if let Some(inner) = after.strip_prefix('[') {
            let Some((count, tail)) = inner.split_once(']') else {
                return;
            };
            arguments = count.trim().parse().unwrap_or(0);
            after = tail;
        }
        if let Some((from, to)) = braced(after, 0) {
            let body = after[from..to].to_string();
            let name = name.trim().to_string();
            project.macros.insert(
                name.clone(),
                Macro {
                    name,
                    arguments,
                    body,
                },
            );
        }
        return;
    }
    if let Some(rest) = text.strip_prefix("\\def\\") {
        let end = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        if let Some((from, to)) = braced(&rest[end..], 0) {
            let name = format!("\\{}", &rest[..end]);
            let body = rest[end..][from..to].to_string();
            project.macros.insert(
                name.clone(),
                Macro {
                    name,
                    arguments: 0,
                    body,
                },
            );
        }
        return;
    }
    for command in ["\\DeclareMathOperator*", "\\DeclareMathOperator"] {
        let Some(rest) = text.strip_prefix(command) else {
            continue;
        };
        let Some((from, to)) = braced(rest, 0) else {
            return;
        };
        let name = rest[from..to].to_string();
        if let Some((a, b)) = braced(rest, to + 1) {
            let body = format!("\\operatorname{{{}}}", &rest[a..b]);
            let arguments = 0;
            project.macros.insert(
                name.clone(),
                Macro {
                    name,
                    arguments,
                    body,
                },
            );
        }
        return;
    }
    if let Some(rest) = text.strip_prefix("\\DeclarePairedDelimiter") {
        let rest = rest.trim_start();
        let (name, tail) = match braced(rest, 0) {
            Some((from, to)) => (rest[from..to].to_string(), &rest[to + 1..]),
            None => {
                let end = rest[1..]
                    .find(|c: char| !c.is_ascii_alphabetic())
                    .map_or(rest.len(), |i| i + 1);
                (rest[..end].to_string(), &rest[end..])
            }
        };
        let Some((a, b)) = braced(tail, 0) else {
            return;
        };
        let Some((c, d)) = braced(tail, b + 1) else {
            return;
        };
        let body = format!("\\left{} #1 \\right{}", &tail[a..b], &tail[c..d]);
        project.macros.insert(
            name.clone(),
            Macro {
                name,
                arguments: 1,
                body,
            },
        );
        return;
    }
    if let Some(rest) = text.strip_prefix("\\newtheorem") {
        let numbered = !rest.starts_with('*');
        let rest = rest.trim_start_matches('*');
        let Some((a, b)) = braced(rest, 0) else {
            return;
        };
        let name = rest[a..b].to_string();
        let mut tail = &rest[b + 1..];
        let mut counter = name.clone();
        if let Some(inner) = tail.strip_prefix('[') {
            let Some((shared, after)) = inner.split_once(']') else {
                return;
            };
            counter = shared.to_string();
            tail = after;
        }
        if let Some((c, d)) = braced(tail, 0) {
            let title = tail[c..d].to_string();
            project.theorems.insert(
                name,
                Theorem {
                    title,
                    counter,
                    numbered,
                },
            );
        }
    }
}

impl Project {
    /// The theorem-like environment called `name`, declared or a familiar one.
    pub fn theorem(&self, name: &str) -> Option<Theorem> {
        let plain = name.trim_end_matches('*');
        if let Some(found) = self.theorems.get(plain) {
            return Some(found.clone());
        }
        let known = THEOREMS.iter().find(|(key, _)| *key == plain)?;
        Some(Theorem {
            title: known.1.to_string(),
            counter: plain.to_string(),
            numbered: !name.ends_with('*'),
        })
    }

    /// Expands the paper's commands in a formula, so the engine sees what they mean.
    pub fn expand(&self, source: &str) -> String {
        let mut text = source.to_string();
        // Macros use macros, but a loop among them must end.
        for _ in 0..8 {
            let next = self.expand_once(&text);
            if next == text {
                break;
            }
            text = next;
        }
        text
    }

    fn expand_once(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find('\\') {
            out.push_str(&rest[..at]);
            rest = &rest[at..];
            let end = rest[1..]
                .find(|c: char| !c.is_ascii_alphabetic())
                .map_or(rest.len(), |i| i + 1);
            let name = &rest[..end.max(2).min(rest.len())];
            let Some(found) = self.macros.get(name) else {
                out.push_str(name);
                rest = &rest[name.len()..];
                continue;
            };
            let mut tail = rest[name.len()..]
                .strip_prefix('*')
                .unwrap_or(&rest[name.len()..]);
            let mut body = found.body.clone();
            for number in 1..=found.arguments {
                tail = tail.trim_start();
                let Some((from, to)) = braced(tail, 0) else {
                    break;
                };
                body = body.replace(&format!("#{number}"), &tail[from..to]);
                tail = &tail[to + 1..];
            }
            out.push_str(&body);
            rest = tail;
        }
        out.push_str(rest);
        out
    }

    /// Reads a paper from its root file, following `\input` as LaTeX would.
    pub fn read(root: &Path) -> Self {
        let mut project = Self::default();
        let mut counters = Counters::default();
        let folder = root.parent().unwrap_or(Path::new("")).to_path_buf();
        project.walk(root, &folder, &mut counters, 0);
        // The ones it names, wherever they are, then any beside it.
        let beside = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "bib"));
        let named = project.bibliographies.clone();
        for path in named.into_iter().chain(beside) {
            if let Ok(text) = std::fs::read_to_string(path) {
                for entry in bib_entries(&text) {
                    project
                        .references
                        .entry(entry.key.clone())
                        .or_insert_with(|| entry.reference());
                    let shown = entry.short();
                    project.citations.entry(entry.key).or_insert(shown);
                }
            }
        }
        project
    }

    /// Reads one file that isn't saved anywhere yet.
    pub fn from_text(text: &str) -> Self {
        let mut project = Self::default();
        let mut counters = Counters::default();
        project.lines(text, Path::new(""), &mut counters, 0);
        project
    }

    fn walk(&mut self, path: &Path, folder: &Path, counters: &mut Counters, depth: usize) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        if depth > 16 {
            return;
        }
        self.starts.insert(path.to_path_buf(), counters.clone());
        self.lines(&text, folder, counters, depth);
        self.after.insert(path.to_path_buf(), counters.clone());
    }

    fn lines(&mut self, text: &str, folder: &Path, counters: &mut Counters, depth: usize) {
        self.setup.extend(setup_of(text));
        self.bibliographies.extend(bibliographies_of(text, folder));
        if let Some(style) = text.lines().find_map(|l| {
            let at = l.find("\\bibliographystyle{")?;
            let rest = &l[at + "\\bibliographystyle{".len()..];
            rest.split('}').next()
        }) {
            self.style = style.trim().to_string();
        }
        for key in cited_in(text) {
            if !self.cited.contains(&key) {
                self.cited.push(key);
            }
        }
        let mut current = String::new();
        let lines: Vec<&str> = text.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let code = line.split('%').next().unwrap_or("");
            read_macro(code, self);
            let trimmed = code.trim_start();
            for command in ["\\input", "\\include"] {
                if let Some(name) =
                    word_after(trimmed, command).filter(|_| trimmed.starts_with(command))
                {
                    let mut file = folder.join(name);
                    if file.extension().is_none() {
                        file.set_extension("tex");
                    }
                    self.walk(&file, folder, counters, depth + 1);
                }
            }
            if trimmed.starts_with("\\appendix") {
                counters.appendix = true;
                counters.sections = [0; 4];
            }
            for (name, level) in [
                ("chapter", 0),
                ("section", 1),
                ("subsection", 2),
                ("subsubsection", 3),
            ] {
                if trimmed.starts_with(&format!("\\{name}{{")) {
                    current = counters.heading(level);
                }
            }
            if let Some(name) =
                word_after(trimmed, "\\begin").filter(|_| trimmed.starts_with("\\begin"))
            {
                match name {
                    "figure" | "figure*" => {
                        counters.figures += 1;
                        current = counters.figures.to_string();
                    }
                    "table" | "table*" => {
                        counters.tables += 1;
                        current = counters.tables.to_string();
                    }
                    _ if is_math(name) => {
                        let close = format!("\\end{{{name}}}");
                        let last = (index..lines.len()).find(|n| lines[*n].contains(&close));
                        if let Some(last) = last {
                            let source = lines[index..=last].join("\n");
                            self.number_equation(&source, !name.ends_with('*'), counters);
                            index = last + 1;
                            continue;
                        }
                    }
                    _ => {
                        if let Some(theorem) = self.theorem(name).filter(|t| t.numbered) {
                            let count = counters.theorems.entry(theorem.counter).or_default();
                            *count += 1;
                            current = count.to_string();
                        }
                    }
                }
            }
            if let Some(label) = word_after(code, "\\label") {
                self.labels.insert(label.to_string(), current.clone());
            }
            index += 1;
        }
    }

    /// Numbers a display, and gives each `\label` in it the number of its own row.
    fn number_equation(&mut self, source: &str, numbered: bool, counters: &mut Counters) {
        let numbers = equation_rows(source, numbered, &mut counters.equations);
        let rows = display_rows(source);
        for (row, number) in rows.iter().zip(numbers) {
            if let (Some(label), Some(number)) = (word_after(row, "\\label"), number) {
                self.labels.insert(label.to_string(), number);
            }
        }
    }
}

/// Environments that are one display of math.
pub fn is_math(name: &str) -> bool {
    let plain = name.trim_end_matches('*');
    [
        "equation",
        "align",
        "gather",
        "multline",
        "displaymath",
        "eqnarray",
        "flalign",
    ]
    .contains(&plain)
}

/// One entry of a bibliography.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BibEntry {
    pub key: String,
    pub title: String,
    pub authors: String,
    pub year: String,
}

impl BibEntry {
    /// `Knuth, Donald (1984). The TeXbook.`, as a references list reads.
    pub fn reference(&self) -> String {
        let authors = self.authors.replace(" and ", "; ");
        let year = if self.year.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.year)
        };
        let title = if self.title.is_empty() {
            String::new()
        } else {
            format!(" {}.", self.title.trim_end_matches('.'))
        };
        let lead = if authors.is_empty() {
            self.key.clone()
        } else {
            authors
        };
        format!("{lead}{year}.{title}")
    }

    /// `Knuth 1984`, as a citation reads on the page.
    pub fn short(&self) -> String {
        let names: Vec<&str> = self.authors.split(" and ").collect();
        let surname = names
            .first()
            .map(|name| surname_of(name))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.key.clone());
        // Two authors read as both, as natbib prints them; three or more as et al.
        let more = match names.len() {
            2 => format!(" and {}", surname_of(names[1])),
            n if n > 2 => " et al.".to_string(),
            _ => String::new(),
        };
        format!("{surname}{more} {}", self.year).trim().to_string()
    }
}

/// What BibTeX takes as a name's family part: all before a comma (`van der Berg, Jan`), or the
/// last word, where a braced group counts as one (`{World Health Organization}`), without the
/// braces that only kept it whole.
fn surname_of(name: &str) -> String {
    let name = name.trim();
    let family = if let Some((family, _)) = split_outside_braces(name, ',') {
        family.trim().to_string()
    } else if name.ends_with('}') {
        // The braced group at the end, whole.
        let mut depth = 0;
        let mut start = 0;
        for (at, c) in name.char_indices().rev() {
            match c {
                '}' => depth += 1,
                '{' => {
                    depth -= 1;
                    if depth == 0 {
                        start = at;
                        break;
                    }
                }
                _ => {}
            }
        }
        name[start..].to_string()
    } else {
        name.split_whitespace()
            .next_back()
            .unwrap_or(name)
            .to_string()
    };
    family.replace(['{', '}'], "")
}

/// Splits at the first `separator` that isn't inside braces.
fn split_outside_braces(text: &str, separator: char) -> Option<(&str, &str)> {
    let mut depth = 0;
    for (at, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ if c == separator && depth == 0 => return Some((&text[..at], &text[at + 1..])),
            _ => {}
        }
    }
    None
}

/// The keys a text cites, in order, each once.
fn cited_in(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in text.lines() {
        let code = line.split('%').next().unwrap_or("");
        let mut rest = code;
        while let Some(at) = rest.find('\\') {
            let after = &rest[at + 1..];
            let name: String = after
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .collect();
            rest = &after[name.len()..];
            if !CITES.contains(&name.as_str()) {
                continue;
            }
            // Past any options, as in `\\citep[see][p.~3]{key}`.
            while rest.starts_with('[') {
                rest = rest.find(']').map_or("", |end| &rest[end + 1..]);
            }
            let Some(inner) = rest.strip_prefix('{').and_then(|r| r.split_once('}')) else {
                continue;
            };
            for key in inner.0.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                if !keys.iter().any(|k| k == key) {
                    keys.push(key.to_string());
                }
            }
            rest = inner.1;
        }
    }
    keys
}

/// The `.bib` files a text names, as paths from `folder`.
fn bibliographies_of(text: &str, folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for line in text.lines() {
        let code = line.split('%').next().unwrap_or("");
        for command in ["\\bibliography{", "\\addbibresource{"] {
            let mut rest = code;
            while let Some(at) = rest.find(command) {
                let after = &rest[at + command.len()..];
                let Some(end) = after.find('}') else {
                    break;
                };
                for name in after[..end]
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    let file = if name.ends_with(".bib") {
                        name.to_string()
                    } else {
                        format!("{name}.bib")
                    };
                    found.push(folder.join(file));
                }
                rest = &after[end..];
            }
        }
    }
    found
}

/// TeX's accents in a `.bib` field as the letters they make: `Cand\\`es` reads "Candès".
fn detex(text: &str) -> String {
    const ACCENTS: [(char, &str, &str); 7] = [
        ('`', "aeiouAEIOU", "àèìòùÀÈÌÒÙ"),
        ('\'', "aeiouyAEIOUY", "áéíóúýÁÉÍÓÚÝ"),
        ('^', "aeiouAEIOU", "âêîôûÂÊÎÔÛ"),
        ('"', "aeiouyAEIOU", "äëïöüÿÄËÏÖÜ"),
        ('~', "anoANO", "ãñõÃÑÕ"),
        ('c', "cC", "çÇ"),
        ('v', "csznrCSZNR", "čšžňřČŠŽŇŘ"),
    ];
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\'
            && let Some(&mark) = chars.get(i + 1)
        {
            // `\\c c` and `\\v s` need a space; the others don't.
            let mut at = i + 2;
            if mark.is_ascii_alphabetic() && chars.get(at) == Some(&' ') {
                at += 1;
            }
            if let Some((_, from, to)) = ACCENTS.iter().find(|(m, _, _)| *m == mark)
                && let Some(&letter) = chars.get(at)
                && let Some(n) = from.chars().position(|c| c == letter)
                && let Some(made) = to.chars().nth(n)
            {
                out.push(made);
                i = at + 1;
                continue;
            }
            let word: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let special = match word.as_str() {
                "ss" => Some("ß"),
                "o" => Some("ø"),
                "O" => Some("Ø"),
                "aa" => Some("å"),
                "AA" => Some("Å"),
                "l" => Some("ł"),
                "L" => Some("Ł"),
                "ae" => Some("æ"),
                _ => None,
            };
            if let Some(special) = special {
                out.push_str(special);
                i += 1 + word.len();
                if chars.get(i) == Some(&' ') {
                    i += 1;
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn field(body: &str, name: &str) -> String {
    let lower = body.to_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        let before = lower[..at].chars().next_back();
        let rest = lower[at + name.len()..].trim_start();
        if before.is_none_or(|c| !c.is_alphanumeric()) && rest.starts_with('=') {
            let value = body[at + name.len()..].trim_start()[1..].trim_start();
            let open = value.chars().next().unwrap_or(' ');
            let text = match open {
                '{' => braced(value, 0).map_or("", |(a, b)| &value[a..b]),
                '"' => value[1..].split('"').next().unwrap_or(""),
                _ => value.split([',', '\n']).next().unwrap_or(""),
            };
            return detex(&text.replace(['{', '}'], "")).trim().to_string();
        }
        from = at + name.len();
    }
    String::new()
}

/// Reads a `.bib` file. Entries can start on one line and take their brace on the next.
pub fn bib_entries(text: &str) -> Vec<BibEntry> {
    let mut entries = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        rest = &rest[at + 1..];
        let Some(open) = rest.find('{') else {
            break;
        };
        let kind = rest[..open].trim().to_lowercase();
        let Some((from, to)) = braced(rest, open) else {
            break;
        };
        let body = &rest[from..to];
        if !["comment", "string", "preamble"].contains(&kind.as_str()) {
            let (key, fields) = body.split_once(',').unwrap_or((body, ""));
            if !key.trim().is_empty() {
                entries.push(BibEntry {
                    key: key.trim().to_string(),
                    title: field(fields, "title"),
                    authors: field(fields, "author"),
                    year: field(fields, "year"),
                });
            }
        }
        rest = &rest[to + 1..];
    }
    entries
}

/// The engines Cellix knows, best first, as commands that take the file last. Each writes the
/// SyncTeX map that links the PDF back to the source.
pub const ENGINES: [&[&str]; 3] = [
    &["tectonic", "-X", "compile", "--synctex", "--keep-logs"],
    &[
        "latexmk",
        "-pdf",
        "-synctex=1",
        "-interaction=nonstopmode",
        "-file-line-error",
    ],
    &[
        "pdflatex",
        "-synctex=1",
        "-interaction=nonstopmode",
        "-file-line-error",
    ],
];

/// The commands that cite, and the ones that refer to a label.
pub const CITES: [&str; 10] = [
    "cite",
    "citep",
    "citet",
    "citeauthor",
    "citeyear",
    "autocite",
    "parencite",
    "textcite",
    "footcite",
    "nocite",
];
pub const REFS: [&str; 6] = ["ref", "eqref", "cref", "Cref", "autoref", "pageref"];

/// What a pdflatex pass left to do, from its `.aux` file: whether BibTeX has to run, and how many
/// more passes it takes for citations and references to settle. Tectonic and latexmk do this
/// themselves.
pub fn more_passes(aux: &str) -> (bool, usize) {
    let bibtex = aux.contains("\\bibdata");
    let passes = if bibtex {
        2
    } else {
        usize::from(aux.contains("\\newlabel") || aux.contains("\\citation"))
    };
    (bibtex, passes)
}

/// Whether a key to a citation or a label is being typed: the text of a line up to the caret
/// ends inside `\cite{` or `\ref{`. Gives which, and the char where the key being typed starts,
/// after any keys before it.
pub fn reference_slot(before: &str) -> Option<(Slot, usize)> {
    let chars: Vec<char> = before.chars().collect();
    let open = chars.iter().rposition(|c| *c == '{')?;
    if chars[open + 1..]
        .iter()
        .any(|c| matches!(c, '}' | '\\' | '{'))
    {
        return None;
    }
    // Past options, as in `\citep[see][p.~3]{`.
    let mut end = open;
    while end > 0 && chars[end - 1] == ']' {
        end = chars[..end].iter().rposition(|c| *c == '[')?;
    }
    let name_start = chars[..end]
        .iter()
        .rposition(|c| !(c.is_ascii_alphabetic() || *c == '*'))?;
    if chars[name_start] != '\\' {
        return None;
    }
    let name: String = chars[name_start + 1..end].iter().collect();
    let name = name.trim_end_matches('*');
    let slot = if CITES.contains(&name) {
        Slot::Cite
    } else if REFS.contains(&name) {
        Slot::Ref
    } else {
        return None;
    };
    let key = chars[open + 1..]
        .iter()
        .rposition(|c| *c == ',')
        .map_or(open + 1, |comma| open + 2 + comma);
    let key = key
        + chars[key..]
            .iter()
            .take_while(|c| c.is_whitespace())
            .count();
    Some((slot, key))
}

/// What goes inside a reference's braces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Cite,
    Ref,
}

/// Finds `file:line: message` in a build's output. Every engine's errors reduce to this.
pub fn problems(output: &str, folder: &Path) -> Vec<Problem> {
    let mut found = Vec::new();
    for line in output.lines() {
        let line = line.strip_prefix("error: ").unwrap_or(line);
        let mut parts = line.splitn(3, ':');
        let (Some(file), Some(number), Some(message)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let file = file.trim().trim_start_matches("./");
        let Ok(number) = number.trim().parse() else {
            continue;
        };
        // Tectonic leaves the extension off an `\input` file.
        let known = [".tex", ".bib", ".sty", ".cls"];
        let name = if known.iter().any(|e| file.ends_with(e)) {
            file.to_string()
        } else if !file.is_empty() && !file.contains(' ') {
            format!("{file}.tex")
        } else {
            continue;
        };
        found.push(Problem {
            file: folder.join(name),
            line: number,
            message: message.trim().to_string(),
        });
    }
    found.dedup();
    found
}

/// Where a build's warnings point, as problems a reader can walk with F8: tectonic's own
/// `warning: file:line: …`, and LaTeX's undefined references and citations, at their line in
/// the file the build started from.
pub fn warning_places(output: &str, folder: &Path, root: &Path) -> Vec<Problem> {
    let mut found = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.trim().strip_prefix("warning: ") {
            found.extend(problems(rest, folder));
            continue;
        }
        if line.contains("undefined on input line")
            && let Some(at) = line.rfind("input line ")
        {
            let number: String = line[at + "input line ".len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(number) = number.parse() {
                let message = line
                    .trim()
                    .trim_start_matches("LaTeX Warning: ")
                    .trim_start_matches("Package natbib Warning: ")
                    .to_string();
                found.push(Problem {
                    file: root.to_path_buf(),
                    line: number,
                    message,
                });
            }
        }
    }
    found.dedup();
    found
}

/// What a build warned about, from its output and log: a build that "worked" can still have
/// unresolved citations and references.
pub fn warnings(output: &str) -> Vec<String> {
    let marks = [
        "warning:",
        "Warning:",
        "undefined on input",
        "There were undefined",
    ];
    let mut found: Vec<String> = output
        .lines()
        .filter(|line| marks.iter().any(|m| line.contains(m)))
        .map(|line| line.trim().to_string())
        .collect();
    found.dedup();
    found
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            other => out.push(other),
        }
    }
    out
}

/// Changes a `tabular`, given its lines from `\begin` to `\end`, and the row and column the
/// caret is in, counting rows of cells. None means the table is gone.
pub fn edit_tabular(text: &str, row: usize, column: usize, edit: Edit) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let is_row = |line: &String| line.contains("\\\\");
    let rows: Vec<usize> = (0..lines.len()).filter(|i| is_row(&lines[*i])).collect();
    let at = *rows.get(row.min(rows.len().checked_sub(1)?))?;
    let cells = |line: &str| line.split("\\\\").next().unwrap_or("").matches('&').count() + 1;
    let columns = cells(&lines[at]);
    let blank = || format!("    {} \\\\", vec![""; columns].join(" & "));
    let spec = |lines: &mut [String], change: &dyn Fn(&mut Vec<char>)| {
        let line = &lines[0];
        let open = line.rfind('{').unwrap_or(0) + 1;
        let close = line.rfind('}').unwrap_or(line.len());
        let mut letters: Vec<char> = line[open..close].chars().collect();
        change(&mut letters);
        let letters: String = letters.into_iter().collect();
        lines[0] = format!("{}{letters}{}", &line[..open], &line[close..]);
    };
    match edit {
        Edit::RowAbove => lines.insert(at, blank()),
        Edit::RowBelow => lines.insert(at + 1, blank()),
        Edit::DeleteRow if rows.len() == 1 => return None,
        Edit::DeleteRow => {
            lines.remove(at);
        }
        Edit::ColumnLeft | Edit::ColumnRight | Edit::DeleteColumn => {
            if edit == Edit::DeleteColumn && columns == 1 {
                return None;
            }
            let right = usize::from(edit == Edit::ColumnRight);
            for index in rows {
                let line = lines[index].clone();
                let (body, _) = line.split_once("\\\\").unwrap_or((&line, ""));
                let mut parts: Vec<String> =
                    body.split('&').map(|c| c.trim().to_string()).collect();
                if edit == Edit::DeleteColumn {
                    if column < parts.len() {
                        parts.remove(column);
                    }
                } else {
                    parts.insert((column + right).min(parts.len()), String::new());
                }
                lines[index] = format!("    {} \\\\", parts.join(" & "));
            }
            spec(&mut lines, &|letters| {
                let spot = (column + right).min(letters.len());
                if edit == Edit::DeleteColumn {
                    if column < letters.len() {
                        letters.remove(column);
                    }
                } else {
                    letters.insert(spot, 'l');
                }
            });
        }
        Edit::Align(align) => {
            let letter = match align {
                Align::Right => 'r',
                Align::Centre => 'c',
                _ => 'l',
            };
            spec(&mut lines, &|letters| {
                if let Some(slot) = letters.get_mut(column) {
                    *slot = letter;
                }
            });
        }
        Edit::Delete => return None,
    }
    Some(lines.join("\n"))
}

/// A new table, ready to fill in.
pub fn new_table(rows: usize, columns: usize) -> String {
    let columns = columns.max(1);
    let row = |text: &dyn Fn(usize) -> String| {
        let cells: Vec<String> = (0..columns).map(text).collect();
        format!("    {} \\\\\n", cells.join(" & "))
    };
    let mut out = format!(
        "\\begin{{table}}[h]\n  \\centering\n  \\begin{{tabular}}{{{}}}\n    \\hline\n",
        "l".repeat(columns)
    );
    out.push_str(&row(&|c| format!("Column {}", c + 1)));
    out.push_str("    \\hline\n");
    for _ in 1..rows.max(2) {
        out.push_str(&row(&|_| String::new()));
    }
    out.push_str("    \\hline\n  \\end{tabular}\n  \\caption{}\n\\end{table}");
    out
}

/// Environments only LaTeX can draw, each built on its own for the page.
pub const DRAWN: [&str; 7] = [
    "tikzpicture",
    "tikzcd",
    "forest",
    "circuitikz",
    "algorithm",
    "algorithmic",
    "algorithm2e",
];

/// What starts a line of setup a piece built alone needs too.
const SETUP: [&str; 10] = [
    "\\usepackage",
    "\\usetikzlibrary",
    "\\usepgfplotslibrary",
    "\\tikzset",
    "\\tikzcdset",
    "\\pgfplotsset",
    "\\newcommand",
    "\\renewcommand",
    "\\DeclareMathOperator",
    "\\def",
];

/// Packages that shape a whole document, which a small picture must not have.
const PAGE_PACKAGES: [&str; 5] = ["geometry", "hyperref", "cleveref", "fancyhdr", "natbib"];

/// The lines of a file's preamble worth repeating, with commands that run over several lines
/// kept whole.
fn setup_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.contains("\\begin{document}") {
            break;
        }
        let code = line.split('%').next().unwrap_or("").trim();
        let wanted = SETUP.iter().any(|start| code.starts_with(start))
            && !PAGE_PACKAGES.iter().any(|name| code.contains(name));
        if !wanted {
            continue;
        }
        let mut whole = code.to_string();
        let open = |text: &str| text.matches('{').count() as i64 - text.matches('}').count() as i64;
        while open(&whole) > 0 {
            let Some(next) = lines.next() else { break };
            whole.push('\n');
            whole.push_str(next.split('%').next().unwrap_or(""));
        }
        out.push(whole);
    }
    out
}

/// A piece of a paper as a file of its own, cut to its size.
pub fn standalone(setup: &[String], body: &str) -> String {
    let mut out = String::from(
        "\\documentclass[border=2pt,varwidth=16cm]{standalone}\n\\usepackage{amsmath,amssymb}\n\\usepackage{tikz}\n",
    );
    if body.contains("tikzcd") {
        out.push_str("\\usepackage{tikz-cd}\n");
    }
    if body.contains("\\begin{algorithmic}") {
        out.push_str("\\usepackage{algpseudocode}\n");
    }
    if body.contains("forest") {
        out.push_str("\\usepackage{forest}\n");
    }
    if body.contains("circuitikz") {
        out.push_str("\\usepackage{circuitikz}\n");
    }
    for line in setup {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\\begin{document}\n");
    out.push_str(body);
    out.push_str("\n\\end{document}\n");
    out
}

/// Where a cell's text starts in a `tabular`'s lines, in chars.
pub fn tabular_cell_offset(text: &str, row: usize, column: usize) -> usize {
    let is_row = |line: &&str| line.contains("\\\\");
    let mut at = 0;
    let mut seen = 0;
    for line in text.split('\n') {
        if is_row(&line) {
            if seen == row {
                let chars: Vec<char> = line.chars().collect();
                let mut start = 0;
                let mut bars = 0;
                while bars < column && start < chars.len() {
                    bars += usize::from(chars[start] == '&');
                    start += 1;
                }
                let spaces = chars[start..].iter().take_while(|c| **c == ' ').count();
                let stops = matches!(chars.get(start + spaces), Some('&' | '\\'));
                return at + start + spaces.min(if stops { 1 } else { spaces });
            }
            seen += 1;
        }
        at += line.chars().count() + 1;
    }
    0
}

const HEADER: &str = "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb,graphicx,hyperref}\n\\usepackage[margin=1in]{geometry}\n\\usepackage{booktabs,xcolor,tikz,tikz-cd}\n\\usepackage[normalem]{ulem}\n\\begin{document}\n";

/// A tag from Markdown's marks, as LaTeX.
fn tag_in_latex(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "<sub>" => "\\textsubscript{",
        "<sup>" => "\\textsuperscript{",
        "<mark>" => "\\colorbox{yellow}{",
        "<ins>" => "\\textcolor{green!50!black}{",
        "<del>" => "\\textcolor{red}{\\sout{",
        "</del>" => "}}",
        "</sub>" | "</sup>" | "</mark>" | "</ins>" => "}",
        _ => return None,
    })
}

/// What each footnote says, by its name.
fn footnote_texts(markdown: &str) -> HashMap<String, String> {
    let definitions = markdown.lines().filter_map(|line| {
        let rest = line.strip_prefix("[^")?;
        let (id, text) = rest.split_once("]:")?;
        Some((id.to_string(), text.trim().to_string()))
    });
    definitions.collect()
}

/// Markdown as a LaTeX document, with diagrams as source.
pub fn from_markdown(markdown: &str) -> String {
    from_markdown_with(markdown, &mut |_, _| None)
}

/// Markdown as a LaTeX document. Math goes through untouched, which is the point. `picture`
/// draws a diagram to a file and names it, since LaTeX can't read one.
pub fn from_markdown_with(
    markdown: &str,
    picture: &mut dyn FnMut(Kind, &str) -> Option<String>,
) -> String {
    let markdown = figure::rewrite_sized(markdown, |image, width| {
        let share = f64::from(width) / 100.0;
        format!("![{}]({} \"{share:.2}\")", image.alt, image.path)
    });
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH;
    let (front, markdown) = crate::front::split(&markdown);
    // A footnote goes in where it's cited, so its text is found first.
    let notes = footnote_texts(markdown);
    let markdown = crate::html::marked(markdown);
    let mut out = String::from(HEADER);
    if !front.is_empty() {
        let field = |value: &Option<String>| value.as_deref().map(escape).unwrap_or_default();
        out.push_str(&format!(
            "\\title{{{}}}\n\\author{{{}}}\n\\date{{{}}}\n\\maketitle\n",
            field(&front.title),
            field(&front.author),
            field(&front.date)
        ));
    }
    let mut verbatim = false;
    let mut columns = 0;
    let mut cell = 0;
    let mut diagram: Option<(Kind, String)> = None;
    let mut tikz: Option<(String, String)> = None;
    let mut skipping = false;
    for event in Parser::new_ext(&markdown, options) {
        if let Some((language, source)) = &mut tikz {
            match event {
                Event::Text(text) => source.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    let body = fenced_tikz(language, source.trim_end());
                    out.push_str(&format!("\n\\begin{{center}}\n{body}\n\\end{{center}}\n"));
                    tikz = None;
                }
                _ => {}
            }
            continue;
        }
        if let Some((kind, source)) = &mut diagram {
            match event {
                Event::Text(text) => source.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    match picture(*kind, source) {
                        Some(file) => out.push_str(&format!(
                            "\n\\begin{{center}}\n\\includegraphics[width=\\linewidth]{{{file}}}\n\\end{{center}}\n"
                        )),
                        None => out.push_str(&format!(
                            "\n\\begin{{verbatim}}\n{source}\\end{{verbatim}}\n"
                        )),
                    }
                    diagram = None;
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Heading { level, .. }) => out.push_str(match level {
                HeadingLevel::H1 => "\n\\section*{",
                HeadingLevel::H2 => "\n\\subsection*{",
                HeadingLevel::H3 => "\n\\subsubsection*{",
                _ => "\n\\paragraph*{",
            }),
            Event::End(TagEnd::Heading(_)) => out.push_str("}\n"),
            Event::Start(Tag::Paragraph) => out.push('\n'),
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::Strong) => out.push_str("\\textbf{"),
            Event::Start(Tag::Emphasis) => out.push_str("\\emph{"),
            Event::Start(Tag::Strikethrough) => out.push_str("\\sout{"),
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
                out.push('}');
            }
            Event::InlineHtml(tag) if tag_in_latex(&tag).is_some() => {
                out.push_str(tag_in_latex(&tag).unwrap_or_default());
            }
            Event::FootnoteReference(id) => {
                let text = notes.get(id.as_ref()).map_or("", String::as_str);
                out.push_str(&format!("\\footnote{{{}}}", escape(text)));
            }
            Event::Start(Tag::FootnoteDefinition(_)) => skipping = true,
            Event::End(TagEnd::FootnoteDefinition) => skipping = false,
            _ if skipping => {}
            Event::Start(Tag::Link { dest_url, .. }) => {
                out.push_str(&format!("\\href{{{}}}{{", escape(&dest_url)));
            }
            Event::End(TagEnd::Link) => out.push('}'),
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                out.push_str(&format!(
                    "\n\\includegraphics[width={title}\\linewidth]{{{dest_url}}}\n\\iffalse "
                ));
            }
            Event::End(TagEnd::Image) => out.push_str("\\fi\n"),
            Event::Start(Tag::BlockQuote(_)) => out.push_str("\n\\begin{quote}"),
            Event::End(TagEnd::BlockQuote(_)) => out.push_str("\\end{quote}\n"),
            Event::Start(Tag::List(None)) => out.push_str("\n\\begin{itemize}\n"),
            Event::Start(Tag::List(Some(_))) => out.push_str("\n\\begin{enumerate}\n"),
            Event::End(TagEnd::List(false)) => out.push_str("\\end{itemize}\n"),
            Event::End(TagEnd::List(true)) => out.push_str("\\end{enumerate}\n"),
            Event::Start(Tag::Item) => out.push_str("\\item "),
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::TaskListMarker(done) => {
                out.push_str(if done {
                    "[$\\boxtimes$] "
                } else {
                    "[$\\square$] "
                });
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if kind_of(&language).is_some() =>
            {
                diagram = kind_of(&language).map(|kind| (kind, String::new()));
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))) if is_tikz(&language) => {
                tikz = Some((language.to_string(), String::new()));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                verbatim = true;
                out.push_str("\n\\begin{verbatim}\n");
            }
            Event::End(TagEnd::CodeBlock) => {
                verbatim = false;
                out.push_str("\\end{verbatim}\n");
            }
            Event::Start(Tag::Table(aligns)) => {
                columns = aligns.len();
                let spec: String = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::Center => 'c',
                        Alignment::Right => 'r',
                        _ => 'l',
                    })
                    .collect();
                out.push_str(&format!("\n\\begin{{tabular}}{{{spec}}}\n\\toprule\n"));
            }
            Event::End(TagEnd::Table) => out.push_str("\\bottomrule\n\\end{tabular}\n"),
            Event::Start(Tag::TableHead | Tag::TableRow) => cell = 0,
            Event::End(TagEnd::TableHead) => out.push_str(" \\\\\n\\midrule\n"),
            Event::End(TagEnd::TableRow) => out.push_str(" \\\\\n"),
            Event::Start(Tag::TableCell) => {
                if cell > 0 && cell < columns {
                    out.push_str(" & ");
                }
                cell += 1;
            }
            Event::Text(text) if verbatim => out.push_str(&text),
            Event::Text(text) => out.push_str(&escape(&text)),
            Event::Code(text) => out.push_str(&format!("\\texttt{{{}}}", escape(&text))),
            Event::InlineMath(text) => out.push_str(&format!("${text}$")),
            Event::DisplayMath(text) => out.push_str(&format!("\n\\[\n{}\n\\]\n", text.trim())),
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("\\\\\n"),
            Event::Rule => out.push_str("\n\\noindent\\rule{\\linewidth}{0.4pt}\n"),
            _ => {}
        }
    }
    out.push_str("\n\\end{document}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_key_is_being_typed_inside_cite_and_ref() {
        assert_eq!(reference_slot("See \\cite{kn"), Some((Slot::Cite, 10)));
        assert_eq!(
            reference_slot("See \\citep[see][p.~3]{a, kn"),
            Some((Slot::Cite, 25))
        );
        assert_eq!(reference_slot("In \\eqref{"), Some((Slot::Ref, 10)));
        assert_eq!(reference_slot("\\textbf{bold"), None);
        assert_eq!(reference_slot("\\ref{done} and"), None);
    }
    #[test]
    fn a_paper_reads_the_bibliography_it_names_and_its_accents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bib")).unwrap();
        std::fs::write(
            dir.path().join("bib/refs.bib"),
            "@article{candes2006, author = {Cand\\`es, Emmanuel and Tao, Terence}, year = 2006}\n",
        )
        .unwrap();
        let root = dir.path().join("main.tex");
        std::fs::write(&root, "\\begin{document}\nSee \\cite{candes2006}.\n\\bibliography{bib/refs}\n\\end{document}\n").unwrap();
        let project = Project::read(&root);
        assert_eq!(
            project.citations.get("candes2006").map(String::as_str),
            Some("Candès and Tao 2006")
        );
        assert_eq!(detex("G\\\"odel, \\c c and \\ss"), "Gödel, ç and ß");
    }
    #[test]
    fn braced_and_particled_names_cite_whole() {
        let entry = |authors: &str| BibEntry {
            key: "k".into(),
            title: String::new(),
            authors: authors.into(),
            year: "2020".into(),
        };
        assert_eq!(
            entry("{World Health Organization}").short(),
            "World Health Organization 2020"
        );
        assert_eq!(entry("van der Berg, Jan").short(), "van der Berg 2020");
        assert_eq!(
            entry("Jan {van der Berg} and Ada Lovelace").short(),
            "van der Berg and Lovelace 2020"
        );
        assert_eq!(entry("Knuth, Donald").short(), "Knuth 2020");
    }

    #[test]
    fn warnings_point_at_their_lines() {
        let root = Path::new("/p/main.tex");
        let output = "warning: sections/results.tex:4: Overfull \\hbox (3pt too wide)\nLaTeX Warning: Reference `sec:x' on page 1 undefined on input line 12.\n";
        let found = warning_places(output, Path::new("/p"), root);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].file, Path::new("/p/sections/results.tex"));
        assert_eq!((found[1].file.as_path(), found[1].line), (root, 12));
        let entry = BibEntry {
            key: "er".into(),
            title: String::new(),
            authors: "Erd\u{151}s, Paul and R\u{e9}nyi, Alfr\u{e9}d".into(),
            year: "1959".into(),
        };
        assert_eq!(entry.short(), "Erd\u{151}s and R\u{e9}nyi 1959");
    }

    #[test]
    fn pdflatex_runs_again_until_references_settle() {
        assert_eq!(more_passes("\\relax\n"), (false, 0));
        assert_eq!(more_passes("\\newlabel{sec:a}{{1}{1}}\n"), (false, 1));
        assert_eq!(more_passes("\\citation{k}\n\\bibdata{refs}\n"), (true, 2));
    }

    #[test]
    fn front_matter_becomes_the_title_block() {
        let tex = from_markdown_with(
            "---\ntitle: The Plan\nauthor: Ada\n---\n\nWords.\n",
            &mut |_, _| None,
        );
        assert!(tex.contains("\\title{The Plan}"), "{tex}");
        assert!(tex.contains("\\maketitle"), "{tex}");
        assert!(!tex.contains("title:"), "{tex}");
    }

    #[test]
    fn a_piece_built_alone_keeps_the_papers_setup_and_not_its_page() {
        let text = "\\documentclass{article}\n\\usepackage{tikz,bm}\n\\usepackage{hyperref}\n\\usepackage[margin=1in]{geometry}\n\\usetikzlibrary{arrows.meta}\n\\tikzset{\n  box/.style={draw}\n}\n\\newcommand{\\R}{\\mathbb{R}} % reals\n\\begin{document}\n\\newcommand{\\later}{x}\n\\end{document}\n";
        let setup = setup_of(text);
        assert_eq!(setup.len(), 4, "{setup:?}");
        assert!(setup[2].contains("box/.style"));
        let tex = standalone(&setup, "\\begin{tikzcd} A \\arrow[r] & B \\end{tikzcd}");
        assert!(tex.contains("\\usepackage{tikz-cd}"));
        assert!(tex.contains("\\newcommand{\\R}"));
        assert!(!tex.contains("geometry"));
        assert!(!tex.contains("later"));
        assert!(tex.ends_with("\\end{document}\n"));
    }

    #[test]
    fn tikz_goes_into_the_paper_as_itself() {
        let tex = from_markdown("```tikz\n\\draw (0,0) -- (1,1);\n```\n");
        assert!(
            tex.contains("\\begin{tikzpicture}\n\\draw (0,0) -- (1,1);\n\\end{tikzpicture}"),
            "{tex}"
        );
        assert!(!tex.contains("verbatim"), "{tex}");
    }

    #[test]
    fn footnotes_and_raised_text_reach_the_paper() {
        let tex = from_markdown("Water H~2~O and x^2^ here[^a].\n\n[^a]: Worth saying.\n");
        assert!(tex.contains("\\textsubscript{2}"), "{tex}");
        assert!(tex.contains("\\textsuperscript{2}"), "{tex}");
        assert!(tex.contains("\\footnote{Worth saying.}"), "{tex}");
        assert_eq!(tex.matches("Worth saying").count(), 1, "{tex}");
    }

    #[test]
    fn a_diagram_and_a_sized_image_reach_the_paper() {
        let text = "```mermaid\nflowchart LR\n  A --> B\n```\n\n![Plot](a.png){width=40%}\n";
        let mut asked = Vec::new();
        let tex = from_markdown_with(text, &mut |kind, source| {
            asked.push((kind, source.to_string()));
            Some("drawn.png".to_string())
        });
        assert_eq!(asked[0].0, Kind::Mermaid);
        assert!(asked[0].1.contains("A --> B"));
        assert!(
            tex.contains("\\includegraphics[width=\\linewidth]{drawn.png}"),
            "{tex}"
        );
        assert!(
            tex.contains("\\includegraphics[width=0.40\\linewidth]{a.png}"),
            "{tex}"
        );
        assert!(!tex.contains("flowchart"), "{tex}");
        let fallback = from_markdown(text);
        assert!(fallback.contains("flowchart"), "{fallback}");
    }

    #[test]
    fn a_tabular_edits_like_a_table() {
        let text = "\\begin{tabular}{lr}\n    \\hline\n    A & B \\\\\n    1 & 2 \\\\\n    \\hline\n\\end{tabular}";
        let more = edit_tabular(text, 1, 0, Edit::RowBelow).unwrap();
        assert_eq!(more.matches("\\\\").count(), 3, "{more}");
        let wider = edit_tabular(text, 0, 0, Edit::ColumnRight).unwrap();
        assert!(wider.contains("{llr}"), "{wider}");
        assert!(wider.contains("A &  & B"), "{wider}");
        let narrower = edit_tabular(text, 0, 1, Edit::DeleteColumn).unwrap();
        assert!(narrower.contains("{l}"), "{narrower}");
        assert!(!narrower.contains('B'), "{narrower}");
        let aligned = edit_tabular(text, 0, 0, Edit::Align(Align::Centre)).unwrap();
        assert!(aligned.contains("{cr}"), "{aligned}");
        let fewer = edit_tabular(text, 1, 0, Edit::DeleteRow).unwrap();
        assert!(!fewer.contains('1'), "{fewer}");
        assert_eq!(edit_tabular(text, 0, 0, Edit::Delete), None);
    }

    #[test]
    fn a_new_table_has_its_size() {
        let table = new_table(3, 2);
        assert!(table.contains("{ll}"));
        assert!(table.contains("Column 1 & Column 2"));
        assert_eq!(table.matches("\\\\\n").count(), 3, "{table}");
    }

    #[test]
    fn errors_are_found_with_their_lines() {
        let output = "note: Running TeX ...\nerror: main.tex:3: Undefined control sequence\nerror: halted on potentially-recoverable error as specified\n./sections/intro.tex:12: Missing $ inserted.\n";
        let found = problems(output, Path::new("/w"));
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].file, Path::new("/w/main.tex"));
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].message, "Undefined control sequence");
        assert_eq!(found[1].file, Path::new("/w/sections/intro.tex"));
    }

    #[test]
    fn a_root_comment_names_the_main_file() {
        let chapter = Path::new("/w/ch/one.tex");
        assert_eq!(root_of(chapter, "\\section{A}\n"), chapter);
        let text = "% !TEX root = ../main.tex\n\\section{A}\n";
        assert_eq!(root_of(chapter, text), Path::new("/w/ch/../main.tex"));
    }

    #[test]
    fn a_bibliography_is_read_however_it_is_laid_out() {
        let bib = "@book\n{knuth84,\n title = {The {TeX}book},\n author = \"Knuth, Donald\",\n year = 1984\n}\n@comment{not this}\n@article{a2, title={B}, author={Ada Lovelace and Bob Smith}, year={2020}}\n";
        let entries = bib_entries(bib);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].key, "knuth84");
        assert_eq!(entries[0].title, "The TeXbook");
        assert_eq!(entries[0].short(), "Knuth 1984");
        assert_eq!(entries[1].short(), "Lovelace and Smith 2020");
    }

    #[test]
    fn errors_in_input_files_keep_their_file() {
        let found = problems(
            "error: sections/two:10: Undefined control sequence\n",
            Path::new("/w"),
        );
        assert_eq!(found[0].file, Path::new("/w/sections/two.tex"));
        assert!(problems("error: halted: 5: nope\n", Path::new("/w")).len() < 2);
    }

    #[test]
    fn a_paper_is_read_across_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("main.tex");
        std::fs::write(
            &root,
            "\\documentclass{article}\n\\input{macros}\n\\begin{document}\n\\section{One}\\label{sec:a}\n\\begin{equation}a\\label{eq:a}\\end{equation}\n\\input{two}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("macros.tex"),
            "\\newcommand{\\R}{\\mathbb{R}}\n\\newcommand{\\KL}[2]{D(#1\\|#2)}\n\\DeclareMathOperator*{\\argmax}{arg\\,max}\n\\DeclarePairedDelimiter{\\norm}{\\lVert}{\\rVert}\n\\newtheorem{thm}{Theorem}\n\\newtheorem{lem}[thm]{Lemma}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("two.tex"),
            "\\section{Two}\\label{sec:b}\n\\begin{align}\na &= b \\label{eq:b}\\\\\nc &= d \\notag\\\\\ne &= f \\label{eq:c}\n\\end{align}\n\\begin{thm}\\label{t:a}x\\end{thm}\n\\begin{lem}\\label{t:b}y\\end{lem}\n",
        )
        .unwrap();
        let project = Project::read(&root);
        assert_eq!(project.labels["sec:b"], "2");
        assert_eq!(project.labels["eq:a"], "1");
        assert_eq!(project.labels["eq:b"], "2", "per row");
        assert_eq!(project.labels["eq:c"], "3", "skipping the notag row");
        assert_eq!(project.labels["t:a"], "1");
        assert_eq!(project.labels["t:b"], "2", "a shared counter");
        let start = &project.starts[&dir.path().join("two.tex")];
        assert_eq!(start.sections[1], 1, "the next file carries on");
        assert_eq!(project.expand(r"x \in \R"), r"x \in \mathbb{R}");
        assert_eq!(project.expand(r"\KL{p}{q}"), r"D(p\|q)");
        assert_eq!(project.expand(r"\norm*{x}"), r"\left\lVert x \right\rVert");
        assert_eq!(project.theorem("lem").unwrap().counter, "thm");
        assert_eq!(project.theorem("proposition").unwrap().title, "Proposition");
        assert_eq!(
            root_of(&dir.path().join("two.tex"), "\\section{Two}\n"),
            root
        );
    }

    #[test]
    fn macros_that_loop_still_end() {
        let mut project = Project::default();
        read_macro(r"\newcommand{\a}{\b}", &mut project);
        read_macro(r"\newcommand{\b}{\a}", &mut project);
        let _ = project.expand(r"\a");
    }

    #[test]
    fn warnings_are_noticed() {
        assert_eq!(
            warnings("note: ok\nwarning: main.tex:3: Citation `x' undefined\n").len(),
            1
        );
        assert!(warnings("note: fine\n").is_empty());
    }

    #[test]
    fn markdown_becomes_latex_with_math_untouched() {
        let tex = from_markdown(
            "# Title\n\nSome **bold** 50% text with $x_i^2$.\n\n$$\\frac{a}{b}$$\n\n| A | B |\n|---|--:|\n| 1 | 2 |\n\n- one\n\n```\ncode_here\n```\n",
        );
        assert!(tex.contains("\\section*{Title}"));
        assert!(tex.contains("\\textbf{bold}"));
        assert!(tex.contains("50\\% text"));
        assert!(tex.contains("$x_i^2$"), "math is left alone: {tex}");
        assert!(tex.contains("\\frac{a}{b}"));
        assert!(tex.contains("\\begin{tabular}{lr}"));
        assert!(tex.contains("1 & 2"));
        assert!(tex.contains("\\begin{itemize}"));
        assert!(tex.contains("code_here"), "verbatim isn't escaped");
    }
}
