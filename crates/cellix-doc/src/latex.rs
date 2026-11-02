//! What a paper needs that isn't on the page: which file to build, what to build it with,
//! what went wrong and where, and Markdown turned into LaTeX for export.

use crate::diagram::{Kind, kind_of};
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
    /// What each citation key shows as: author and year.
    pub citations: HashMap<String, String>,
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
fn display_rows(source: &str) -> Vec<&str> {
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
        for path in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = path.path();
            if path.extension().is_some_and(|e| e == "bib")
                && let Ok(text) = std::fs::read_to_string(path)
            {
                for entry in bib_entries(&text) {
                    let shown = entry.short();
                    project.citations.insert(entry.key, shown);
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
    }

    fn lines(&mut self, text: &str, folder: &Path, counters: &mut Counters, depth: usize) {
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
    /// `Knuth 1984`, as a citation reads on the page.
    pub fn short(&self) -> String {
        let first = self.authors.split(" and ").next().unwrap_or("");
        let surname = first
            .split(',')
            .next()
            .and_then(|name| name.split_whitespace().next_back())
            .unwrap_or(&self.key);
        let many = self.authors.contains(" and ");
        let more = if many { " et al." } else { "" };
        format!("{surname}{more} {}", self.year).trim().to_string()
    }
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
            return text.replace(['{', '}'], "").trim().to_string();
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

/// The engines Cellix knows, best first, as commands that take the file last.
pub const ENGINES: [&[&str]; 3] = [
    &["tectonic", "-X", "compile", "--synctex", "--keep-logs"],
    &[
        "latexmk",
        "-pdf",
        "-interaction=nonstopmode",
        "-file-line-error",
    ],
    &["pdflatex", "-interaction=nonstopmode", "-file-line-error"],
];

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

const HEADER: &str = "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb,graphicx,hyperref}\n\\usepackage[margin=1in]{geometry}\n\\usepackage{booktabs}\n\\usepackage[normalem]{ulem}\n\\begin{document}\n";

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
        | Options::ENABLE_MATH;
    let mut out = String::from(HEADER);
    let mut verbatim = false;
    let mut columns = 0;
    let mut cell = 0;
    let mut diagram: Option<(Kind, String)> = None;
    for event in Parser::new_ext(&markdown, options) {
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
        assert_eq!(entries[1].short(), "Lovelace et al. 2020");
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
