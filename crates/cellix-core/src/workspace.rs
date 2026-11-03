use crate::pattern::{Options, char_ranges};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// One visible row of the file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

/// One line containing a project search hit. Lines count from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    pub line: usize,
    /// Char offset of the hit within `text`.
    pub column: usize,
    /// Chars the hit covers, or none for a place found some other way.
    pub len: usize,
    pub text: String,
}

/// Search stops here, because results past this help nobody and cost memory.
const MAX_MATCHES: usize = 5000;
/// Larger files are skipped: they're rarely source, and reading them stalls a search.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// The root folder and which of its directories are expanded.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded: BTreeSet::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
    }

    pub fn expand(&mut self, dir: &Path) {
        self.expanded.insert(dir.to_path_buf());
    }

    /// Visible rows: directories first, then files. Hidden and ignored files show, as in
    /// VS Code, because agents often work in them. Only `.git` stays out, and what LaTeX leaves
    /// beside a paper.
    pub fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        self.collect(&self.root, 0, &mut out);
        out
    }

    fn collect(&self, dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        let mut children: Vec<(PathBuf, bool)> = WalkBuilder::new(dir)
            .max_depth(Some(1))
            .standard_filters(false)
            .filter_entry(|e| e.file_name() != ".git" && e.file_name() != ".DS_Store")
            .build()
            .flatten()
            .filter(|e| e.depth() == 1)
            .map(|e| {
                let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                (e.into_path(), is_dir)
            })
            .collect();
        children.retain(|(path, is_dir)| *is_dir || !is_build_leftover(path));
        children.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.file_name().cmp(&b.0.file_name()))
        });
        for (path, is_dir) in children {
            let expanded = is_dir && self.expanded.contains(&path);
            out.push(Entry {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: path.clone(),
                depth,
                is_dir,
                expanded,
            });
            if expanded {
                self.collect(&path, depth + 1, out);
            }
        }
    }

    /// All files under the root, for quick open and project search.
    pub fn files(&self) -> Vec<PathBuf> {
        WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .filter_entry(|e| e.file_name() != ".git")
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(ignore::DirEntry::into_path)
            .collect()
    }

    /// Every place `query` matches, in path order, one for each match on a line. Binary files
    /// are skipped.
    pub fn search(&self, query: &str, options: Options) -> Vec<Match> {
        let mut files = self.files();
        files.sort();
        let mut found = Vec::new();
        let Ok(matcher) = options.matcher(query) else {
            return found;
        };
        for path in files {
            if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (line, content) in text.lines().enumerate() {
                for (start, end) in char_ranges(&matcher, content, MAX_MATCHES) {
                    found.push(Match {
                        path: path.clone(),
                        line,
                        column: start,
                        len: end - start,
                        text: content.to_string(),
                    });
                    if found.len() >= MAX_MATCHES {
                        return found;
                    }
                }
            }
        }
        found
    }

    /// Replaces `query` in every file except `skip`, and returns how many changed.
    pub fn replace_all(
        &self,
        query: &str,
        with: &str,
        options: Options,
        skip: &[PathBuf],
    ) -> io::Result<usize> {
        let mut changed = 0;
        let Ok(matcher) = options.matcher(query) else {
            return Ok(changed);
        };
        for path in self.files() {
            if skip.contains(&path) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if matcher.is_match(&text) {
                fs::write(&path, options.replace_all(&matcher, &text, with))?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    pub fn create_file(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(drop)
    }

    pub fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if to.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "target already exists",
            ));
        }
        fs::rename(from, to)
    }

    /// Deletes a file or a whole directory. Refuses anything outside the root.
    pub fn delete(&self, path: &Path) -> io::Result<()> {
        if !path.starts_with(&self.root) || path == self.root {
            return Err(io::Error::other("refusing to delete outside the workspace"));
        }
        if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    }
}

/// What a LaTeX build writes beside the paper it built: no one opens these.
const LEFTOVERS: [&str; 13] = [
    ".aux",
    ".log",
    ".out",
    ".toc",
    ".bbl",
    ".blg",
    ".fls",
    ".fdb_latexmk",
    ".synctex.gz",
    ".lof",
    ".lot",
    ".nav",
    ".snm",
];

/// Whether this is one of those, with its paper beside it.
fn is_build_leftover(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    LEFTOVERS.iter().any(|ending| {
        name.strip_suffix(ending)
            .is_some_and(|stem| path.with_file_name(format!("{stem}.tex")).exists())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_papers_build_files_stay_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "paper.tex",
            "paper.aux",
            "paper.synctex.gz",
            "paper.pdf",
            "notes.log",
        ] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let workspace = Workspace::new(dir.path().to_path_buf());
        let names: Vec<String> = workspace.entries().into_iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            ["notes.log", "paper.pdf", "paper.tex"],
            "a log with no paper stays"
        );
    }

    #[test]
    fn a_project_search_lists_every_match_and_replaces_with_its_options() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "foo Foo food\n").unwrap();
        let workspace = Workspace::new(dir.path().to_path_buf());
        let found = workspace.search("foo", Options::default());
        assert_eq!(found.len(), 3, "each match on the line");
        assert_eq!((found[1].column, found[1].len), (4, 3));
        let whole = Options {
            whole: true,
            case: true,
            ..Options::default()
        };
        assert_eq!(workspace.search("foo", whole).len(), 1);
        workspace.replace_all("foo", "bar", whole, &[]).unwrap();
        let text = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(text, "bar Foo food\n");
    }
}
