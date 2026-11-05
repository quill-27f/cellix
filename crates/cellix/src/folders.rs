//! Finding a folder to open as a workspace: suggestions before anything is typed, and the
//! folders inside a path as it's typed, so it can be walked down with Tab.

use std::path::{Path, PathBuf};

use crate::palette::Item;

/// What the row of a folder says about it.
pub const RUNNING: &str = "running";
pub const STOPPED: &str = "stopped";
const BESIDE: &str = "beside this one";
const IN_HOME: &str = "in home";
/// The folder a typed path names, offered first.
pub const THIS_FOLDER: &str = "this folder";
/// Enough to choose from without walking anything big.
const MOST: usize = 400;

/// What the folder picker knows, and what it last listed.
pub struct Picker {
    /// The project the picker was opened from.
    root: PathBuf,
    home: Option<PathBuf>,
    /// Other projects with work on this machine, and whether it's running or stopped.
    running: Vec<(PathBuf, &'static str)>,
    /// The typed folder whose contents are listed, `None` for the suggestions, and whether
    /// hidden folders are among them.
    listed: Option<(Option<String>, bool)>,
}

impl Picker {
    pub fn new(
        root: PathBuf,
        home: Option<PathBuf>,
        running: Vec<(PathBuf, &'static str)>,
    ) -> Self {
        Self {
            root,
            home,
            running,
            listed: None,
        }
    }

    /// The rows for what's typed, or `None` when those listed already fit it and only need
    /// filtering. A path lists the folders in it; anything else, the suggestions.
    pub fn items_for(&mut self, query: &str) -> Option<Vec<Item>> {
        let folder = typed_folder(query);
        let hidden = folder
            .as_ref()
            .is_some_and(|typed| query[typed.len().min(query.len())..].starts_with('.'));
        let key = (folder.clone(), hidden);
        if self.listed.as_ref() == Some(&key) {
            return None;
        }
        self.listed = Some(key);
        Some(match folder {
            Some(typed) => self.inside(&typed, query),
            None => self.suggestions(),
        })
    }

    /// The folder a typed path means: from home with `~`, from the project with `./` and
    /// `../`, as itself when absolute, and otherwise from beside the project, where sibling
    /// projects are, or else inside it.
    pub fn resolve(&self, typed: &str) -> Option<PathBuf> {
        let typed = typed.trim();
        if typed.is_empty() {
            return None;
        }
        let path = if typed == "~" {
            self.home.clone()?
        } else if let Some(rest) = typed.strip_prefix("~/") {
            self.home.as_ref()?.join(rest)
        } else if Path::new(typed).is_absolute() {
            PathBuf::from(typed)
        } else if typed.starts_with("./") || typed.starts_with("../") {
            self.root.join(typed)
        } else {
            let beside = self.root.parent().map(|p| p.join(typed));
            beside
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| self.root.join(typed))
        };
        dunce::canonicalize(path).ok().filter(|p| p.is_dir())
    }

    /// The folders in what's typed so far, each written as it would be typed, so the query
    /// still matches it. The folder itself comes first.
    fn inside(&self, typed: &str, query: &str) -> Vec<Item> {
        let Some(folder) = self.resolve(typed) else {
            return Vec::new();
        };
        let hidden = query
            .get(typed.len()..)
            .is_some_and(|rest| rest.starts_with('.'));
        let mut items = vec![Item::Folder(
            folder.clone(),
            // As typed, slash and all, so what's typed still matches it.
            typed.to_string(),
            THIS_FOLDER,
        )];
        for name in subfolders(&folder, hidden) {
            items.push(Item::Folder(
                folder.join(&name),
                format!("{typed}{name}"),
                "",
            ));
        }
        items
    }

    /// Before anything is typed: other projects with work running, the folders beside this
    /// project, and those in home, each group apart.
    fn suggestions(&self) -> Vec<Item> {
        let mut seen = vec![self.root.clone()];
        let mut items = Vec::new();
        for (path, note) in &self.running {
            if !seen.contains(path) {
                items.push(Item::Folder(path.clone(), self.written(path), note));
                seen.push(path.clone());
            }
        }
        let mut groups = Vec::new();
        let beside = self.root.parent().map_or_else(Vec::new, |parent| {
            subfolders(parent, false)
                .into_iter()
                .map(|name| parent.join(name))
                .collect()
        });
        groups.push((beside, BESIDE));
        let home = self.home.as_ref().map_or_else(Vec::new, |home| {
            subfolders(home, false)
                .into_iter()
                .map(|name| home.join(name))
                .collect()
        });
        groups.push((home, IN_HOME));
        for (paths, note) in groups {
            let fresh: Vec<PathBuf> = paths
                .into_iter()
                .filter(|path| !seen.contains(path))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            if !items.is_empty() {
                items.push(Item::Gap);
            }
            for path in fresh {
                if items.len() >= MOST {
                    break;
                }
                items.push(Item::Folder(path.clone(), self.written(&path), note));
                seen.push(path);
            }
        }
        items
    }

    /// A path as a person would type it: from home with `~`.
    fn written(&self, path: &Path) -> String {
        match self
            .home
            .as_ref()
            .and_then(|home| path.strip_prefix(home).ok())
        {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }
}

/// The folder part of what's typed, up to and including its last slash, when it's a path:
/// `~/src/ap` gives `~/src/`. A bare name is a search, and gives `None`.
fn typed_folder(query: &str) -> Option<String> {
    if query == "~" {
        return Some("~/".to_string());
    }
    let pathlike = query.contains('/');
    if !pathlike {
        return None;
    }
    let end = query.rfind('/')? + 1;
    Some(query[..end].to_string())
}

/// The folders directly in `folder`, by name, sorted, leaving out hidden ones unless asked.
fn subfolders(folder: &Path, hidden: bool) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| hidden || !name.starts_with('.'))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.truncate(MOST);
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Folder(_, label, "") => label.clone(),
                Item::Folder(_, label, note) => format!("{label} ({note})"),
                Item::Gap => "--".to_string(),
                _ => "?".to_string(),
            })
            .collect()
    }

    #[test]
    fn a_typed_path_lists_the_folders_in_it_and_bare_names_search() {
        assert_eq!(typed_folder("api"), None);
        assert_eq!(typed_folder("~"), Some("~/".to_string()));
        assert_eq!(typed_folder("~/src/ap"), Some("~/src/".to_string()));
        assert_eq!(typed_folder("/srv/"), Some("/srv/".to_string()));
        assert_eq!(typed_folder("../"), Some("../".to_string()));
    }

    #[test]
    fn suggestions_then_folders_inside_what_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dunce::canonicalize(dir.path()).unwrap();
        let src = home.join("src");
        for folder in [
            "src/cellix",
            "src/api-server/crates",
            "src/infra",
            "notes",
            ".cache",
        ] {
            fs::create_dir_all(home.join(folder)).unwrap();
        }
        let running = vec![(src.join("infra"), RUNNING)];
        let mut picker = Picker::new(src.join("cellix"), Some(home.clone()), running);

        // Nothing typed: what's running, then what's beside this project, then home, and
        // never this project itself or anything hidden.
        let items = picker.items_for("").unwrap();
        assert_eq!(
            labels(&items),
            [
                "~/src/infra (running)",
                "--",
                "~/src/api-server (beside this one)",
                "--",
                "~/notes (in home)",
                "~/src (in home)",
            ]
        );
        // A name filters the same list, so it isn't listed again.
        assert!(picker.items_for("api").is_none());

        // A path lists what's in it, written as typed, after the folder itself.
        let items = picker.items_for("~/src/").unwrap();
        assert_eq!(
            labels(&items),
            [
                "~/src/ (this folder)",
                "~/src/api-server",
                "~/src/cellix",
                "~/src/infra"
            ]
        );
        assert!(picker.items_for("~/src/ap").is_none(), "only filtered");
        let items = picker.items_for("~/src/api-server/").unwrap();
        assert_eq!(
            labels(&items),
            ["~/src/api-server/ (this folder)", "~/src/api-server/crates"]
        );
        // Relative paths start from the project; a missing folder lists nothing.
        let items = picker.items_for("../").unwrap();
        assert!(
            labels(&items).contains(&"../infra".to_string()),
            "{:?}",
            labels(&items)
        );
        assert!(picker.items_for("~/nowhere/").unwrap().is_empty());
        // Hidden folders only when asked for.
        let items = picker.items_for("~/.").unwrap();
        assert!(labels(&items).contains(&"~/.cache".to_string()));
    }
}
