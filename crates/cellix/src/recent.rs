//! The folders and files opened lately, newest first, for Open Recent and the welcome page.
//! One list for the machine, kept beside the sessions, so every Cellix adds to it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::palette::Item;

/// As many of each as are kept.
const KEPT: usize = 30;

#[derive(Default, Serialize, Deserialize)]
struct Lists {
    #[serde(default)]
    folders: Vec<PathBuf>,
    #[serde(default)]
    files: Vec<PathBuf>,
}

/// The lists, and the file they're kept in.
#[derive(Default)]
pub struct Recent {
    /// `None` keeps them in memory only, as unit tests do.
    file: Option<PathBuf>,
    lists: Lists,
}

impl Recent {
    /// The machine's lists. Unit tests run on the real cache, so they keep their own.
    pub fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let file = crate::hold::sessions_base()
            .parent()
            .map(|cache| cache.join("recent.json"));
        let mut recent = Self {
            file,
            lists: Lists::default(),
        };
        recent.reread();
        recent
    }

    /// Takes in what other Cellix processes added since.
    fn reread(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(file)
            && let Ok(lists) = serde_json::from_str(&text)
        {
            self.lists = lists;
        }
    }

    fn write(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.lists) {
            // Beside it and renamed, so another Cellix never reads half a file.
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }

    pub fn opened_folder(&mut self, path: &Path) {
        self.reread();
        put_first(&mut self.lists.folders, path);
        self.write();
    }

    pub fn opened_file(&mut self, path: &Path) {
        self.reread();
        put_first(&mut self.lists.files, path);
        self.write();
    }

    /// Folders still there, newest first, leaving out `except`.
    pub fn folders(&mut self, except: &Path) -> Vec<PathBuf> {
        self.reread();
        let folders = self.lists.folders.iter();
        folders
            .filter(|p| p.as_path() != except && p.is_dir())
            .cloned()
            .collect()
    }

    /// Files still there, newest first.
    pub fn files(&mut self) -> Vec<PathBuf> {
        self.reread();
        self.lists
            .files
            .iter()
            .filter(|p| p.is_file())
            .cloned()
            .collect()
    }

    /// The newest few, folders first, as last read, without touching the disk but to see
    /// they're still there: for the welcome page, which draws every frame.
    pub fn newest(&self, except: &Path, most: usize) -> Vec<(PathBuf, bool)> {
        let folders = self
            .lists
            .folders
            .iter()
            .filter(|p| p.as_path() != except && p.is_dir());
        let files = self.lists.files.iter().filter(|p| p.is_file());
        folders
            .map(|p| (p.clone(), true))
            .chain(files.map(|p| (p.clone(), false)))
            .take(most)
            .collect()
    }

    /// The palette's rows: folders, a gap, then files, each as `~` writes it.
    pub fn items(&mut self, root: &Path, home: Option<&Path>) -> Vec<Item> {
        let mut items: Vec<Item> = self
            .folders(root)
            .into_iter()
            .map(|path| {
                let written = written(&path, home);
                Item::Recent(path, written, true)
            })
            .collect();
        let files: Vec<Item> = self
            .files()
            .into_iter()
            .map(|path| {
                let written = written(&path, home);
                Item::Recent(path, written, false)
            })
            .collect();
        if !items.is_empty() && !files.is_empty() {
            items.push(Item::Gap);
        }
        items.extend(files);
        items
    }
}

fn put_first(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(KEPT);
}

/// A path as it's typed: from home with `~`.
pub fn written(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(inside) if inside.as_os_str().is_empty() => "~".to_string(),
        // With `/` between the parts on every system, as `~` itself is written.
        Some(inside) => {
            let parts: Vec<String> = inside
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!("~/{}", parts.join("/"))
        }
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_once_each_and_kept_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent.json");
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut recent = Recent {
            file: Some(file.clone()),
            lists: Lists::default(),
        };
        recent.opened_folder(&a);
        recent.opened_folder(&b);
        recent.opened_folder(&a);
        assert_eq!(recent.folders(Path::new("/")), vec![a.clone(), b.clone()]);
        // Another Cellix sees the same list.
        let mut other = Recent {
            file: Some(file),
            lists: Lists::default(),
        };
        assert_eq!(other.folders(&a), vec![b]);
        assert_eq!(written(&a.join("x"), Some(dir.path())), "~/a/x");
    }
}
