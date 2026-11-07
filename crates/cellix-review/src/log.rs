//! The history of the repository and of one file: commits, the files each changed, and the
//! lanes that draw them as a graph.

use std::path::{Path, PathBuf};

use crate::git::{Change, git, parse_name_status};

/// One commit, as a history list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Seconds since the epoch, as the author dated it.
    pub time: i64,
    pub summary: String,
    /// Branches and tags pointing at it, `HEAD -> ` taken off.
    pub refs: Vec<String>,
    /// For a file's history, what the file was called in this commit.
    pub path: Option<PathBuf>,
}

impl Commit {
    pub fn short(&self) -> &str {
        self.hash.get(..7).unwrap_or(&self.hash)
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

const FIELDS: &str = "%H%x00%P%x00%an%x00%at%x00%D%x00%s";

fn parse(output: &str) -> Vec<Commit> {
    output
        .split('\x1e')
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(6, '\0');
            let hash = fields.next()?.trim().to_string();
            let parents = fields
                .next()?
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let author = fields.next()?.to_string();
            let time = fields.next()?.trim().parse().unwrap_or(0);
            let refs = fields
                .next()?
                .split(", ")
                .map(|r| r.trim_start_matches("HEAD -> ").trim_start_matches("tag: "))
                .filter(|r| !r.is_empty() && *r != "HEAD")
                .map(str::to_string)
                .collect();
            // The subject, then for a file's history a blank line and the file's name.
            let rest = fields.next().unwrap_or_default();
            let (summary, names) = rest.split_once('\n').unwrap_or((rest, ""));
            let path = names
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .map(PathBuf::from);
            Some(Commit {
                hash,
                parents,
                author,
                time,
                summary: summary.to_string(),
                refs,
                path,
            })
        })
        .collect()
}

/// The newest commits, parents after their children. `all` takes in every branch, not only
/// the one checked out.
pub fn log(root: &Path, limit: usize, all: bool) -> Vec<Commit> {
    let format = format!("--format=%x1e{FIELDS}");
    let count = format!("-n{limit}");
    let mut args = vec!["log", "--topo-order", &format, &count];
    if all {
        args.extend(["--branches", "--remotes", "--tags", "HEAD"]);
    }
    git(root, &args).map_or_else(Vec::new, |o| parse(&o))
}

/// Commits that changed one file, newest first, following it through renames.
pub fn file_history(root: &Path, path: &Path, limit: usize) -> Vec<Commit> {
    let format = format!("--format=%x1e{FIELDS}");
    let count = format!("-n{limit}");
    let file = path.to_string_lossy();
    let args = [
        "log",
        "--follow",
        "--name-only",
        &format,
        &count,
        "--",
        &file,
    ];
    git(root, &args).map_or_else(Vec::new, |o| parse(&o))
}

/// The files a commit changed, against its first parent.
pub fn commit_files(root: &Path, commit: &Commit) -> Vec<Change> {
    let output = match commit.parents.first() {
        Some(parent) => git(
            root,
            &[
                "diff",
                "--no-renames",
                "--name-status",
                "-z",
                parent,
                &commit.hash,
            ],
        ),
        None => git(
            root,
            &[
                "diff-tree",
                "-r",
                "--root",
                "--no-commit-id",
                "--name-status",
                "-z",
                &commit.hash,
            ],
        ),
    };
    output.map_or_else(Vec::new, |o| parse_name_status(&o))
}

/// A file before and after a commit: as its first parent had it, and as the commit left it.
pub fn commit_sides(root: &Path, commit: &Commit, path: &Path) -> (String, String) {
    let after = crate::text_at(root, &commit.hash, path);
    let before = commit
        .parents
        .first()
        .map(|p| crate::text_at(root, p, path))
        .unwrap_or_default();
    (before, after)
}

/// How long ago, in a few letters: "now", "4 min", "3 hr", "yesterday", "5 days", "2 mo".
pub fn age(time: i64, now: i64) -> String {
    let seconds = (now - time).max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match () {
        _ if minutes < 1 => "now".to_string(),
        _ if hours < 1 => format!("{minutes} min"),
        _ if days < 1 => format!("{hours} hr"),
        _ if days < 2 => "yesterday".to_string(),
        _ if days < 14 => format!("{days} days"),
        _ if days < 60 => format!("{} wk", days / 7),
        _ if days < 365 => format!("{} mo", days / 30),
        _ => format!("{} yr", days / 365),
    }
}

/// How one commit's row of the graph is drawn. Lanes are columns; each is a straight line,
/// and lines meet a commit's dot along its row, so cells and the paint layer can both draw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// The lane the commit's dot sits in.
    pub lane: usize,
    /// Lanes with a line in the top half of the row, coming down from newer commits.
    pub above: Vec<bool>,
    /// Lanes with a line in the bottom half, going on to older ones.
    pub below: Vec<bool>,
    /// Other lanes that end at this commit: branches whose next commit is this one.
    pub joins: Vec<usize>,
    /// Lanes leaving this commit for its other parents.
    pub forks: Vec<usize>,
}

impl GraphRow {
    pub fn width(&self) -> usize {
        self.above.len().max(self.below.len()).max(self.lane + 1)
    }

    /// The lanes a line runs across, along the row, from the dot.
    pub fn across(&self) -> Option<(usize, usize)> {
        let ends = self.joins.iter().chain(&self.forks).copied();
        let low = ends.clone().chain([self.lane]).min()?;
        let high = ends.chain([self.lane]).max()?;
        (low != high).then_some((low, high))
    }
}

/// Lays commits out in lanes. Each lane waits for one commit; a commit takes the lane waiting
/// for it, or the first free one, and hands it on to its first parent.
pub fn graph(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<&str>> = Vec::new();
    let free = |lanes: &mut Vec<Option<&str>>, not: Option<usize>| {
        let open = (0..lanes.len()).find(|&i| lanes[i].is_none() && Some(i) != not);
        open.unwrap_or_else(|| {
            lanes.push(None);
            lanes.len() - 1
        })
    };
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let above: Vec<bool> = lanes.iter().map(Option::is_some).collect();
        let waiting: Vec<usize> = (0..lanes.len())
            .filter(|&i| lanes[i] == Some(commit.hash.as_str()))
            .collect();
        let lane = match waiting.first() {
            Some(&lane) => lane,
            None => free(&mut lanes, None),
        };
        let joins: Vec<usize> = waiting.iter().skip(1).copied().collect();
        for &join in &joins {
            lanes[join] = None;
        }
        lanes[lane] = commit.parents.first().map(String::as_str);
        let mut forks = Vec::new();
        for parent in commit.parents.iter().skip(1) {
            let existing = lanes.iter().position(|l| *l == Some(parent.as_str()));
            let to = existing.unwrap_or_else(|| {
                let to = free(&mut lanes, Some(lane));
                lanes[to] = Some(parent.as_str());
                to
            });
            if to != lane {
                forks.push(to);
            }
        }
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
        let below = lanes.iter().map(Option::is_some).collect();
        rows.push(GraphRow {
            lane,
            above,
            below,
            joins,
            forks,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict::tests::run;
    use crate::git::tests::repo;
    use std::fs;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            ..Commit::default()
        }
    }

    #[test]
    fn a_branch_and_its_merge_take_two_lanes() {
        // m merges b into a; a and b both come from base.
        let commits = [
            commit("m", &["a", "b"]),
            commit("b", &["base"]),
            commit("a", &["base"]),
            commit("base", &[]),
        ];
        let rows = graph(&commits);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].forks, vec![1]);
        assert_eq!(rows[0].below, vec![true, true]);
        assert_eq!(rows[0].across(), Some((0, 1)));
        assert_eq!(rows[1].lane, 1, "b sits in the lane the merge opened");
        assert_eq!(rows[1].above, vec![true, true]);
        assert_eq!(rows[2].lane, 0);
        // Both lanes wait for base, so the second ends there.
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[3].joins, vec![1]);
        assert_eq!(rows[3].below, Vec::<bool>::new());
        assert_eq!(rows[3].width(), 2);
    }

    #[test]
    fn a_straight_history_is_one_lane() {
        let commits = [commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        let rows = graph(&commits);
        assert!(rows.iter().all(|r| r.lane == 0 && r.width() == 1));
        assert_eq!(rows[0].above, Vec::<bool>::new());
        assert_eq!(rows[1].above, vec![true]);
    }

    #[test]
    fn the_log_reads_a_real_repository() {
        let dir = repo();
        let path = dir.path();
        assert!(run(path, &["checkout", "-q", "-b", "side"]));
        fs::write(path.join("side.txt"), "s\n").unwrap();
        assert!(run(path, &["add", "."]));
        assert!(run(path, &["commit", "-qm", "On the side"]));
        assert!(run(path, &["checkout", "-q", "-"]));
        fs::write(path.join("kept.txt"), "two\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Second"]));
        assert!(run(path, &["merge", "-q", "--no-edit", "side"]));
        let commits = log(path, 50, true);
        assert_eq!(commits.len(), 4);
        assert!(commits[0].is_merge());
        assert!(commits[0].summary.starts_with("Merge branch 'side'"));
        assert!(
            commits[0].refs.iter().any(|r| r == "master" || r == "main"),
            "{:?}",
            commits[0].refs
        );
        assert_eq!(commits[0].author, "Tester");
        let rows = graph(&commits);
        assert_eq!(rows.iter().map(GraphRow::width).max(), Some(2));
        // The side commit's files, against its parent.
        let side = commits.iter().find(|c| c.summary == "On the side").unwrap();
        let files = commit_files(path, side);
        assert_eq!(files.len(), 1);
        assert_eq!(
            (files[0].path.as_path(), files[0].status),
            (Path::new("side.txt"), 'A')
        );
        assert_eq!(
            commit_sides(path, side, Path::new("side.txt")),
            (String::new(), "s\n".into())
        );
        let first = commits.last().unwrap();
        assert!(first.parents.is_empty());
        assert_eq!(commit_files(path, first).len(), 2);
    }

    #[test]
    fn a_files_history_follows_it_through_a_rename() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("kept.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Grow"]));
        assert!(run(path, &["mv", "kept.txt", "moved.txt"]));
        assert!(run(path, &["commit", "-qm", "Move"]));
        fs::write(path.join("moved.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Five"]));
        let history = file_history(path, Path::new("moved.txt"), 50);
        let names: Vec<_> = history.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(names, ["Five", "Move", "Grow", "first"]);
        assert_eq!(history[0].path.as_deref(), Some(Path::new("moved.txt")));
        assert_eq!(history[3].path.as_deref(), Some(Path::new("kept.txt")));
        let (before, after) = commit_sides(path, &history[0], Path::new("moved.txt"));
        assert!(!before.contains("five") && after.contains("five"));
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(age(1000, 1000), "now");
        assert_eq!(age(0, 4 * 60), "4 min");
        assert_eq!(age(0, 3 * 3600), "3 hr");
        assert_eq!(age(0, 30 * 3600), "yesterday");
        assert_eq!(age(0, 5 * 86400), "5 days");
        assert_eq!(age(0, 400 * 86400), "1 yr");
    }
}
