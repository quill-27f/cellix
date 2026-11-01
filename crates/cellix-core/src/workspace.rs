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

/// The root folder and which of its directories are expanded.
#[derive(Debug)]
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

    /// Visible rows: directories first, then files, honouring .gitignore.
    pub fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        self.collect(&self.root, 0, &mut out);
        out
    }

    fn collect(&self, dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        let mut children: Vec<(PathBuf, bool)> = WalkBuilder::new(dir)
            .max_depth(Some(1))
            .hidden(false)
            .require_git(false)
            .filter_entry(|e| e.file_name() != ".git")
            .build()
            .flatten()
            .filter(|e| e.depth() == 1)
            .map(|e| {
                let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                (e.into_path(), is_dir)
            })
            .collect();
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
