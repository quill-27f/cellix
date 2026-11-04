//! A LaTeX paper split over files, read as LaTeX reads it: each `\input` and `\include`
//! followed from the root's folder, and a root found only if it really takes a file in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// How many folders above a file its root is looked for in.
const LEVELS_UP: usize = 3;

/// How deep `\input` files are followed, past any loop.
const DEEPEST: usize = 16;

/// A line up to its `%` comment, if it has one.
pub fn uncommented(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (at, b) in bytes.iter().enumerate() {
        if *b == b'%' && (at == 0 || bytes[at - 1] != b'\\') {
            return &line[..at];
        }
    }
    line
}

/// The next `\input{name}`, `\input name` or `\include{name}` in a line without its comment:
/// where it starts, the name, and where it ends. `\includegraphics` and the like aren't one.
pub fn next_input(line: &str) -> Option<(usize, &str, usize)> {
    let mut from = 0;
    while let Some(found) = line[from..].find('\\') {
        let at = from + found;
        let after = &line[at + 1..];
        let len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        if len == 0 {
            // An escaped character, as in `\\` or `\%`.
            from = at + 1 + after.chars().next().map_or(0, char::len_utf8);
            continue;
        }
        from = at + 1 + len;
        let command = &after[..len];
        if command != "input" && command != "include" {
            continue;
        }
        let rest = &line[from..];
        let gap = rest.len() - rest.trim_start().len();
        let open = from + gap;
        if line[open..].starts_with('{') {
            let Some(close) = line[open..].find('}').map(|c| open + c) else {
                continue;
            };
            return Some((at, line[open + 1..close].trim(), close + 1));
        }
        // TeX's own `\input name` reads the name to the next space.
        if command == "input" && gap > 0 {
            let len = line[open..]
                .find(|c: char| c.is_whitespace() || c == '}' || c == '\\')
                .unwrap_or(line.len() - open);
            if len > 0 {
                return Some((at, &line[open..open + len], open + len));
            }
        }
    }
    None
}

/// Every `\input` and `\include` in a line without its comment, in order.
pub fn inputs(line: &str) -> Vec<(usize, &str, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some((at, name, end)) = next_input(&line[from..]) {
        found.push((from + at, name, from + end));
        from += end;
    }
    found
}

/// The file an `\input` names, from the folder LaTeX runs in: `name.tex` when there is one,
/// as LaTeX tries first, or else `name` as written.
pub fn resolve(folder: &Path, name: &str) -> PathBuf {
    let plain = folder.join(name);
    if name.ends_with(".tex") {
        return plain;
    }
    let tex = folder.join(format!("{name}.tex"));
    if !tex.is_file() && plain.is_file() {
        plain
    } else {
        tex
    }
}

/// Whether two paths are the same file, however they're written.
pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// A paper's files as read from the disk, each kept until its size or time of change does, so
/// counting a long paper again as one file is typed in costs a look at each, not a read.
#[derive(Default)]
pub struct Files {
    canonical: HashMap<PathBuf, PathBuf>,
    texts: HashMap<PathBuf, (Stamp, Rc<str>)>,
}

/// When a file last changed, and its size.
type Stamp = (Option<std::time::SystemTime>, u64);

impl Files {
    /// A file's text, read again only once it has changed.
    pub fn text(&mut self, path: &Path) -> Option<Rc<str>> {
        let meta = std::fs::metadata(path).ok()?;
        let stamp = (meta.modified().ok(), meta.len());
        if let Some((known, text)) = self.texts.get(path)
            && *known == stamp
            && stamp.0.is_some()
        {
            return Some(text.clone());
        }
        let text: Rc<str> = std::fs::read_to_string(path).ok()?.into();
        self.texts.insert(path.to_path_buf(), (stamp, text.clone()));
        Some(text)
    }

    /// The one way a file's path is written, so the same file is known however it's named.
    pub fn canonical(&mut self, path: &Path) -> PathBuf {
        if let Some(known) = self.canonical.get(path) {
            return known.clone();
        }
        match path.canonicalize() {
            Ok(canonical) => {
                self.canonical.insert(path.to_path_buf(), canonical.clone());
                canonical
            }
            Err(_) => path.to_path_buf(),
        }
    }
}

/// A paper with every file it takes in read into its place.
pub struct Expanded {
    pub text: String,
    /// Each file read, once, as [`Files::canonical`] writes it.
    pub files: Vec<PathBuf>,
}

/// Reads a paper from its root, with each `\input` and `\include` read in where it stands,
/// as LaTeX does from the root's folder. A file in `open` is read from the text given there,
/// as an editor holds it unsaved, not from the disk. A file that takes itself in, however
/// indirectly, is read once.
pub fn expand(root: &Path, open: &[(&Path, &str)], files: &mut Files) -> Expanded {
    let open: Vec<(PathBuf, &str)> = open
        .iter()
        .map(|(path, text)| (files.canonical(path), *text))
        .collect();
    let mut walk = Walk {
        folder: root.parent().unwrap_or(Path::new("")),
        open: &open,
        files,
        read: Vec::new(),
        stack: Vec::new(),
    };
    let text = walk.expand(root);
    Expanded {
        text,
        files: walk.read,
    }
}

struct Walk<'a> {
    folder: &'a Path,
    open: &'a [(PathBuf, &'a str)],
    files: &'a mut Files,
    read: Vec<PathBuf>,
    /// The files being read in, outermost first, so a loop is seen.
    stack: Vec<PathBuf>,
}

impl Walk<'_> {
    fn expand(&mut self, path: &Path) -> String {
        let canonical = self.files.canonical(path);
        if self.stack.contains(&canonical) || self.stack.len() > DEEPEST {
            return String::new();
        }
        let own: Rc<str> = match self.open.iter().find(|(p, _)| *p == canonical) {
            Some((_, text)) => Rc::from(*text),
            None => match self.files.text(path) {
                Some(text) => text,
                None => return String::new(),
            },
        };
        if !self.read.contains(&canonical) {
            self.read.push(canonical.clone());
        }
        self.stack.push(canonical);
        let mut out = String::with_capacity(own.len());
        for line in own.lines() {
            let mut rest = uncommented(line);
            while let Some((at, name, end)) = next_input(rest) {
                out.push_str(&rest[..at]);
                out.push('\n');
                let file = resolve(self.folder, name);
                out.push_str(&self.expand(&file));
                out.push('\n');
                rest = &rest[end..];
            }
            out.push_str(rest);
            out.push('\n');
        }
        self.stack.pop();
        out
    }
}

/// Whether a text is a whole document, with a `\documentclass` that isn't commented out.
pub fn is_whole(text: &str) -> bool {
    text.lines()
        .any(|line| uncommented(line).contains("\\documentclass"))
}

/// The whole document that takes `path` in, looked for in its folder and a few above it, no
/// higher than a repository's top. Of several in one folder, the first by name.
pub fn root_including(path: &Path) -> Option<PathBuf> {
    let mut files = Files::default();
    let target = files.canonical(path);
    let mut place = path.parent();
    for _ in 0..=LEVELS_UP {
        let Some(folder) = place else {
            break;
        };
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "tex") && p != path)
            .collect();
        candidates.sort();
        for candidate in candidates {
            let whole = files.text(&candidate).is_some_and(|text| is_whole(&text));
            if whole
                && files.canonical(&candidate) != target
                && expand(&candidate, &[], &mut files).files.contains(&target)
            {
                return Some(candidate);
            }
        }
        if folder.join(".git").exists() {
            break;
        }
        place = folder.parent().filter(|p| !p.as_os_str().is_empty());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_found_however_they_are_written() {
        let names = |line: &str| -> Vec<String> {
            inputs(line).iter().map(|(_, n, _)| n.to_string()).collect()
        };
        assert_eq!(names("\\input{a} and \\include {b}"), ["a", "b"]);
        assert_eq!(names("\\input intro\\par"), ["intro"]);
        assert_eq!(
            names("Text \\input sections/two.tex more"),
            ["sections/two.tex"]
        );
        assert!(names("\\includegraphics{fig} \\inputencoding{utf8}").is_empty());
        assert!(names("\\\\input{a} \\includeonly{b}").is_empty());
        assert!(names("\\input").is_empty());
        assert!(names("\\input{unclosed").is_empty());
    }

    #[test]
    fn a_papers_files_are_read_again_only_once_they_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("intro.tex");
        std::fs::write(&path, "one").unwrap();
        let mut files = Files::default();
        assert_eq!(files.text(&path).as_deref(), Some("one"));
        // Rewritten behind its back with the same size and time: still the copy it kept.
        let then = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "two").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(then).unwrap();
        assert_eq!(files.text(&path).as_deref(), Some("one"), "not read again");
        let later = then + std::time::Duration::from_secs(2);
        file.set_modified(later).unwrap();
        assert_eq!(
            files.text(&path).as_deref(),
            Some("two"),
            "changed, so read"
        );
        std::fs::write(&path, "three!").unwrap();
        file.set_modified(later).unwrap();
        assert_eq!(files.text(&path).as_deref(), Some("three!"), "a new size");
    }

    #[test]
    fn a_name_is_tried_with_tex_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ch.v2.tex"), "").unwrap();
        std::fs::write(dir.path().join("fig.tikz"), "").unwrap();
        assert_eq!(resolve(dir.path(), "ch.v2"), dir.path().join("ch.v2.tex"));
        assert_eq!(resolve(dir.path(), "fig.tikz"), dir.path().join("fig.tikz"));
        assert_eq!(
            resolve(dir.path(), "intro.tex"),
            dir.path().join("intro.tex")
        );
        assert_eq!(
            resolve(dir.path(), "missing"),
            dir.path().join("missing.tex")
        );
    }
}
