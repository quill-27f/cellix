//! Tasks, as VS Code's Run Task finds them: package.json scripts, Cargo, Makefile targets, a
//! justfile, Go, pytest and `.vscode/tasks.json`. And the errors in what they print, read
//! into file, line and message for the Problems list.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where a task was found, which is how the picker groups it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// `.vscode/tasks.json`, which the person wrote, so it comes first.
    Configured,
    Cargo,
    Npm,
    Make,
    Just,
    Go,
    Pytest,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Self::Configured => "tasks.json",
            Self::Cargo => "Cargo",
            Self::Npm => "npm",
            Self::Make => "Makefile",
            Self::Just => "justfile",
            Self::Go => "Go",
            Self::Pytest => "pytest",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// What the picker and the pane's tab call it.
    pub label: String,
    /// What runs, through the shell.
    pub command: String,
    /// Where it runs.
    pub folder: PathBuf,
    pub source: Source,
    /// In VS Code's build group, which Ctrl+Shift+B runs.
    pub build: bool,
    /// The build task Ctrl+Shift+B runs without asking.
    pub default_build: bool,
}

impl Task {
    fn new(label: &str, command: String, folder: &Path, source: Source) -> Self {
        Self {
            label: label.to_string(),
            command,
            folder: folder.to_path_buf(),
            source,
            build: false,
            default_build: false,
        }
    }
}

/// Every task in the project at `root`, configured ones first.
pub fn find(root: &Path) -> Vec<Task> {
    let mut tasks = configured(root);
    tasks.extend(cargo(root));
    tasks.extend(npm(root));
    for sub in subfolders(root) {
        tasks.extend(npm(&sub));
    }
    tasks.extend(make(root));
    tasks.extend(just(root));
    tasks.extend(go(root));
    tasks.extend(pytest(root));
    tasks
}

/// The task Ctrl+Shift+B runs: the one marked default, or the only build task there is.
pub fn default_build(tasks: &[Task]) -> Option<&Task> {
    if let Some(task) = tasks.iter().find(|t| t.default_build) {
        return Some(task);
    }
    let builds: Vec<&Task> = tasks.iter().filter(|t| t.build).collect();
    (builds.len() == 1).then(|| builds[0])
}

/// Folders one level down that hold a project of their own, as a web app beside a server.
fn subfolders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            name.is_some_and(|n| !n.starts_with('.') && n != "node_modules" && n != "target")
        })
        .filter(|p| p.join("package.json").is_file())
        .collect();
    found.sort();
    found
}

fn cargo(root: &Path) -> Vec<Task> {
    if !root.join("Cargo.toml").is_file() {
        return Vec::new();
    }
    let mut tasks: Vec<Task> = ["build", "test", "run", "clippy"]
        .iter()
        .map(|verb| {
            Task::new(
                &format!("cargo {verb}"),
                format!("cargo {verb}"),
                root,
                Source::Cargo,
            )
        })
        .collect();
    tasks[0].build = true;
    tasks
}

fn npm(folder: &Path) -> Vec<Task> {
    let Ok(text) = std::fs::read_to_string(folder.join("package.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let manager = if folder.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if folder.join("yarn.lock").is_file() {
        "yarn"
    } else if folder.join("bun.lockb").is_file() || folder.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    };
    scripts
        .keys()
        .map(|name| {
            let command = format!("{manager} run {name}");
            let mut task = Task::new(&command, command.clone(), folder, Source::Npm);
            task.build = name == "build";
            task
        })
        .collect()
}

fn make(root: &Path) -> Vec<Task> {
    let file = ["GNUmakefile", "makefile", "Makefile"]
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file());
    let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) else {
        return Vec::new();
    };
    let mut targets: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some((names, rest)) = line.split_once(':') else {
            continue;
        };
        // `x := y` and `x ::= y` are variables, not targets.
        if rest.starts_with('=') || rest.starts_with(":=") || names.contains(['=', '$', '%']) {
            continue;
        }
        for name in names.split_whitespace() {
            if !targets.iter().any(|t| t == name) {
                targets.push(name.to_string());
            }
        }
    }
    let mut tasks: Vec<Task> = targets
        .iter()
        .map(|t| {
            Task::new(
                &format!("make {t}"),
                format!("make {t}"),
                root,
                Source::Make,
            )
        })
        .collect();
    let build = tasks
        .iter()
        .position(|t| t.label == "make build" || t.label == "make all")
        .or((!tasks.is_empty()).then_some(0));
    if let Some(index) = build {
        tasks[index].build = true;
    }
    tasks
}

fn just(root: &Path) -> Vec<Task> {
    let file = ["justfile", "Justfile", ".justfile"]
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file());
    let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) else {
        return Vec::new();
    };
    let mut tasks = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '[']) || line.starts_with("set ") {
            continue;
        }
        let line = line.strip_prefix('@').unwrap_or(line);
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        if rest.starts_with('=') || head.starts_with("alias ") || head.starts_with("export ") {
            continue;
        }
        let Some(name) = head.split_whitespace().next() else {
            continue;
        };
        if name.starts_with('_')
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || "-_".contains(c))
        {
            continue;
        }
        let mut task = Task::new(
            &format!("just {name}"),
            format!("just {name}"),
            root,
            Source::Just,
        );
        task.build = name == "build";
        tasks.push(task);
    }
    tasks
}

fn go(root: &Path) -> Vec<Task> {
    if !root.join("go.mod").is_file() {
        return Vec::new();
    }
    let mut build = Task::new("go build", "go build ./...".into(), root, Source::Go);
    build.build = true;
    let test = Task::new("go test", "go test ./...".into(), root, Source::Go);
    vec![build, test]
}

fn pytest(root: &Path) -> Vec<Task> {
    let said =
        |name: &str| std::fs::read_to_string(root.join(name)).is_ok_and(|t| t.contains("pytest"));
    let uses = root.join("pytest.ini").is_file()
        || root.join("conftest.py").is_file()
        || said("pyproject.toml")
        || said("setup.cfg")
        || said("tox.ini");
    if !uses {
        return Vec::new();
    }
    vec![Task::new(
        "pytest",
        "python -m pytest".into(),
        root,
        Source::Pytest,
    )]
}

/// `.vscode/tasks.json`, which is JSON with comments and trailing commas.
fn configured(root: &Path) -> Vec<Task> {
    let Ok(text) = std::fs::read_to_string(root.join(".vscode/tasks.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&plain_json(&text)) else {
        return Vec::new();
    };
    let Some(list) = json.get("tasks").and_then(Value::as_array) else {
        return Vec::new();
    };
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let mut tasks = Vec::new();
    for entry in list {
        // A platform's own settings take the place of the general ones.
        let mut entry = entry.clone();
        if let (Some(own), Some(map)) = (entry.get(os).cloned(), entry.as_object_mut())
            && let Some(own) = own.as_object()
        {
            for (key, value) in own {
                map.insert(key.clone(), value.clone());
            }
        }
        let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
        let kind = text("type").unwrap_or_else(|| "shell".into());
        let command = if kind == "npm" {
            text("script").map(|s| format!("npm run {s}"))
        } else {
            text("command").map(|command| {
                let args = entry.get("args").and_then(Value::as_array);
                let args = args.into_iter().flatten().filter_map(|a| match a {
                    Value::String(s) => Some(quoted(s)),
                    Value::Object(o) => o.get("value").and_then(Value::as_str).map(quoted),
                    _ => None,
                });
                std::iter::once(command)
                    .chain(args)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        };
        let Some(command) = command else {
            continue;
        };
        let label = text("label").unwrap_or_else(|| command.clone());
        let folder = entry
            .pointer("/options/cwd")
            .and_then(Value::as_str)
            .map_or_else(|| root.to_path_buf(), |cwd| root.join(expand(cwd, root)));
        let (build, default_build) = match entry.get("group") {
            Some(Value::String(group)) => (group == "build", false),
            Some(Value::Object(group)) => {
                let build = group.get("kind").and_then(Value::as_str) == Some("build");
                let default = group
                    .get("isDefault")
                    .is_some_and(|d| d != &Value::Bool(false));
                (build, build && default)
            }
            _ => (false, false),
        };
        let mut task = Task::new(&label, expand(&command, root), &folder, Source::Configured);
        task.build = build;
        task.default_build = default_build;
        tasks.push(task);
    }
    tasks
}

/// VS Code's variables that make sense outside it.
fn expand(text: &str, root: &Path) -> String {
    let root = root.display().to_string();
    let name = Path::new(&root)
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    text.replace("${workspaceFolder}", &root)
        .replace("${workspaceRoot}", &root)
        .replace("${workspaceFolderBasename}", &name)
        .replace("${cwd}", &root)
}

fn quoted(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_alphanumeric() || "-_./=:,@%+${}".contains(c))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// JSON with comments and trailing commas made plain JSON.
fn plain_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            (',', _) => {
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some(']' | '}')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// A problem a task printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    /// From 1, as tools print it.
    pub line: usize,
    pub column: usize,
    pub message: String,
    /// 1 is an error, 2 a warning, as the language servers say.
    pub severity: u64,
}

/// The errors and warnings in what a task printed, from rustc and Cargo, tsc, ESLint,
/// gcc and clang, Go and pytest. Paths are taken from `folder`, where the task ran.
pub fn problems(output: &str, folder: &Path) -> Vec<Problem> {
    let text = without_escapes(output);
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut found: Vec<Problem> = Vec::new();
    // ESLint's stylish output names a file on a line of its own, then its problems.
    let mut eslint_file: Option<PathBuf> = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // rustc: `error[E0308]: message`, then `  --> src/x.rs:17:24` a line or two later.
        if let Some((severity, message)) = rust_head(line) {
            let arrow = lines[i + 1..]
                .iter()
                .take(3)
                .find_map(|l| l.trim_start().strip_prefix("--> "));
            if let Some((file, line_no, column)) = arrow.and_then(file_line_column) {
                found.push(Problem {
                    file: place(folder, file),
                    line: line_no,
                    column,
                    message: message.to_string(),
                    severity,
                });
            }
            i += 1;
            continue;
        }
        if let Some(problem) = tsc(line, folder)
            .or_else(|| compiler(line, folder))
            .or_else(|| python_place(line, &lines[i + 1..], folder))
            .or_else(|| pytest_failed(line, folder))
            .or_else(|| go_test(line, folder))
        {
            found.push(problem);
            i += 1;
            continue;
        }
        let trimmed = line.trim();
        if !line.starts_with(char::is_whitespace) && looks_like_path(trimmed) {
            eslint_file = Some(place(folder, trimmed));
        } else if let Some(file) = &eslint_file
            && let Some(problem) = eslint_row(trimmed, file)
        {
            found.push(problem);
        } else if trimmed.is_empty() {
            eslint_file = None;
        }
        i += 1;
    }
    // Cargo says a warning once per target it builds: one row is enough.
    let mut seen = Vec::new();
    found.retain(|p| {
        let key = (p.file.clone(), p.line, p.column, p.message.clone());
        let fresh = !seen.contains(&key);
        seen.push(key);
        fresh
    });
    found
}

/// Takes out colour and other terminal escapes, and carriage-return overwrites.
fn without_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn rust_head(line: &str) -> Option<(u64, &str)> {
    let (severity, rest) = if let Some(rest) = line.strip_prefix("error") {
        (1, rest)
    } else {
        (2, line.strip_prefix("warning")?)
    };
    let rest = match rest.strip_prefix('[') {
        Some(code) => &code[code.find(']')? + 1..],
        None => rest,
    };
    let message = rest.strip_prefix(": ")?;
    // Cargo's summaries aren't about a place.
    let summary = message.starts_with("could not compile")
        || message.contains("warnings emitted")
        || message.contains("warning emitted")
        || message.starts_with("aborting due to");
    (!summary).then_some((severity, message))
}

/// `path:line:column`, or `path:line`.
fn file_line_column(text: &str) -> Option<(&str, usize, usize)> {
    let mut parts = text.trim().rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    match (middle.parse::<usize>(), last.parse::<usize>(), parts.next()) {
        (Ok(line), Ok(column), Some(file)) if !file.is_empty() => Some((file, line, column)),
        _ => {
            let line = last.parse().ok()?;
            let file = text.trim().rsplit_once(':')?.0;
            Some((file, line, 1))
        }
    }
}

/// tsc: `src/a.ts(12,5): error TS2322: message`, or `src/a.ts:12:5 - error TS2322: message`.
fn tsc(line: &str, folder: &Path) -> Option<Problem> {
    let (file, line_no, column, rest) = if let Some(open) = line.find('(')
        && line[open..].contains("): ")
    {
        let close = open + line[open..].find(')')?;
        let (l, c) = line[open + 1..close].split_once(',')?;
        (
            &line[..open],
            l.parse().ok()?,
            c.parse().ok()?,
            &line[close + 3..],
        )
    } else {
        let (place, rest) = line.split_once(" - ")?;
        let (file, l, c) = file_line_column(place)?;
        (file, l, c, rest)
    };
    let (severity, rest) = if let Some(rest) = rest.strip_prefix("error ") {
        (1, rest)
    } else {
        (2, rest.strip_prefix("warning ")?)
    };
    let (code, message) = rest.split_once(": ")?;
    if !code.starts_with("TS") || file.contains(' ') {
        return None;
    }
    Some(Problem {
        file: place(folder, file),
        line: line_no,
        column,
        message: message.to_string(),
        severity,
    })
}

/// gcc, clang, Go and ESLint's unix format: `file:line:col: error: message`, or for Go
/// `./main.go:12:5: message`.
fn compiler(line: &str, folder: &Path) -> Option<Problem> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let mut parts = line.splitn(4, ':');
    let file = parts.next()?;
    let line_no: usize = parts.next()?.trim().parse().ok()?;
    let third = parts.next()?;
    let (column, rest) = match third.trim().parse::<usize>() {
        Ok(column) => (column, parts.next()?.trim_start()),
        Err(_) => (1, line.splitn(3, ':').nth(2)?.trim_start()),
    };
    if file.is_empty() || file.contains(' ') || !looks_like_path(file) {
        return None;
    }
    let (severity, message) = if let Some(m) = rest.strip_prefix("fatal error: ") {
        (1, m)
    } else if let Some(m) = rest.strip_prefix("error: ") {
        (1, m)
    } else if let Some(m) = rest.strip_prefix("warning: ") {
        (2, m)
    } else if rest.starts_with("note: ") {
        return None;
    } else if file.ends_with(".go") {
        (1, rest)
    } else {
        // ESLint's unix format puts the kind last: `message [Error/rule]`.
        let kind = rest.rfind(" [")?;
        let severity = if rest[kind..].starts_with(" [Warning") {
            2
        } else {
            1
        };
        (severity, &rest[..kind])
    };
    Some(Problem {
        file: place(folder, file),
        line: line_no,
        column,
        message: message.to_string(),
        severity,
    })
}

/// A Python traceback's frame, `  File "x.py", line 12, in f`, or pytest's
/// `tests/test_x.py:12: AssertionError` under a failure. The message is the error after it.
fn python_place(line: &str, after: &[&str], folder: &Path) -> Option<Problem> {
    if let Some(rest) = line.trim_start().strip_prefix("File \"") {
        let (file, rest) = rest.split_once('"')?;
        let number = rest.strip_prefix(", line ")?;
        let number: usize = number.split(',').next()?.trim().parse().ok()?;
        // Library frames aren't where the fix goes.
        if file.contains("site-packages") || file.starts_with('<') {
            return None;
        }
        let message = after
            .iter()
            .find(|l| !l.starts_with(' ') && l.contains("Error"))
            .map_or_else(|| "raised here".to_string(), |l| l.trim().to_string());
        return Some(Problem {
            file: place(folder, file),
            line: number,
            column: 1,
            message,
            severity: 1,
        });
    }
    // pytest: no space before the path, and an error after the line number.
    let (file, rest) = line.split_once(".py:")?;
    let (number, message) = rest.split_once(": ")?;
    let number: usize = number.parse().ok()?;
    if file.contains(' ') || !message.contains("Error") && !message.starts_with("in ") {
        return None;
    }
    if message.starts_with("in ") {
        return None;
    }
    Some(Problem {
        file: place(folder, &format!("{file}.py")),
        line: number,
        column: 1,
        message: message.to_string(),
        severity: 1,
    })
}

/// pytest's summary: `FAILED tests/test_x.py::test_name - AssertionError: message`. It
/// names no line, so the test's file has a row at its top, unless a line was found.
fn pytest_failed(line: &str, folder: &Path) -> Option<Problem> {
    let rest = line.strip_prefix("FAILED ")?;
    let (test, message) = rest.split_once(" - ").unwrap_or((rest, "failed"));
    let (file, name) = test.split_once("::")?;
    Some(Problem {
        file: place(folder, file),
        line: 1,
        column: 1,
        message: format!("{name}: {message}"),
        severity: 1,
    })
}

/// `go test`: `    main_test.go:12: message`, indented under the test.
fn go_test(line: &str, folder: &Path) -> Option<Problem> {
    let trimmed = line.trim_start();
    if trimmed.len() == line.len() {
        return None;
    }
    let (file, rest) = trimmed.split_once(".go:")?;
    let (number, message) = rest.split_once(": ")?;
    Some(Problem {
        file: place(folder, &format!("{file}.go")),
        line: number.parse().ok()?,
        column: 1,
        message: message.to_string(),
        severity: 1,
    })
}

/// ESLint's stylish row: `  12:5  error  message  rule-name`.
fn eslint_row(row: &str, file: &Path) -> Option<Problem> {
    let (place, rest) = row.split_once(char::is_whitespace)?;
    let (line_no, column) = place.split_once(':')?;
    let rest = rest.trim_start();
    let (severity, rest) = if let Some(rest) = rest.strip_prefix("error") {
        (1, rest)
    } else {
        (2, rest.strip_prefix("warning")?)
    };
    // The rule's name is last, two spaces after the message.
    let rest = rest.trim();
    let message = rest
        .rsplit_once("  ")
        .map_or(rest, |(message, _)| message.trim());
    Some(Problem {
        file: file.to_path_buf(),
        line: line_no.parse().ok()?,
        column: column.parse().ok()?,
        message: message.to_string(),
        severity,
    })
}

/// A path a tool printed, from where it ran.
fn place(folder: &Path, file: &str) -> PathBuf {
    folder.join(file.strip_prefix("./").unwrap_or(file))
}

fn looks_like_path(text: &str) -> bool {
    let name = text.rsplit('/').next().unwrap_or(text);
    name.contains('.') && !text.contains(char::is_whitespace) && !text.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(problems: &[Problem]) -> Vec<(String, usize, usize, u64)> {
        problems
            .iter()
            .map(|p| {
                let file = p.file.strip_prefix("/p").unwrap().display().to_string();
                (file, p.line, p.column, p.severity)
            })
            .collect()
    }

    #[test]
    fn cargo_errors_and_warnings_with_their_places() {
        let out = "\u{1b}[0m\u{1b}[1m\u{1b}[38;5;9merror[E0308]\u{1b}[0m: mismatched types\n  \
                   --> src/queue.rs:17:24\n   |\nwarning: unused variable: `limit`\n \
                   --> src/lib.rs:4:9\nwarning: unused variable: `limit`\n --> src/lib.rs:4:9\n\
                   error: could not compile `queue` (lib) due to 1 previous error\n";
        let found = problems(out, Path::new("/p"));
        assert_eq!(
            at(&found),
            vec![
                ("src/queue.rs".into(), 17, 24, 1),
                ("src/lib.rs".into(), 4, 9, 2)
            ]
        );
        assert_eq!(found[0].message, "mismatched types");
    }

    #[test]
    fn tsc_eslint_gcc_go_and_pytest() {
        let out = "src/a.ts(12,5): error TS2322: Type 'string' is not assignable.\n\
                   src/b.ts:3:1 - error TS1005: ';' expected.\n\
                   \n/p/web/c.js\n  7:10  error  'x' is not defined  no-undef\n  \
                   9:1   warning  Unexpected console  no-console\n\n\
                   main.c:4:12: error: expected ';' after expression\n\
                   main.c:4:12: note: here\n\
                   ./cmd/main.go:8:2: undefined: foo\n\
                   --- FAIL: TestX (0.00s)\n    x_test.go:21: got 1, want 2\n\
                   tests/test_q.py:14: AssertionError\n\
                   FAILED tests/test_q.py::test_full - assert 1 == 2\n";
        let found = problems(out, Path::new("/p"));
        assert_eq!(
            at(&found),
            vec![
                ("src/a.ts".into(), 12, 5, 1),
                ("src/b.ts".into(), 3, 1, 1),
                ("web/c.js".into(), 7, 10, 1),
                ("web/c.js".into(), 9, 1, 2),
                ("main.c".into(), 4, 12, 1),
                ("cmd/main.go".into(), 8, 2, 1),
                ("x_test.go".into(), 21, 1, 1),
                ("tests/test_q.py".into(), 14, 1, 1),
                ("tests/test_q.py".into(), 1, 1, 1),
            ]
        );
        assert_eq!(found[2].message, "'x' is not defined");
        assert_eq!(found[6].message, "got 1, want 2");
    }

    #[test]
    fn tasks_from_every_kind_of_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            root.join("Makefile"),
            ".PHONY: all\nCC := cc\nall: build\nbuild: main.o\n\tcc -o x main.o\n%.o: %.c\n",
        )
        .unwrap();
        std::fs::write(
            root.join("justfile"),
            "set shell := [\"sh\"]\nx := '1'\n@lint arg:\n  echo\n_hidden:\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(
            root.join("web/package.json"),
            r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("web/pnpm-lock.yaml"), "").unwrap();
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        std::fs::write(
            root.join(".vscode/tasks.json"),
            r#"{
              // comments are allowed
              "version": "2.0.0",
              "tasks": [
                {"label": "Docs", "type": "shell", "command": "mdbook",
                 "args": ["build", "my book"], "options": {"cwd": "${workspaceFolder}/docs"},
                 "group": {"kind": "build", "isDefault": true},},
              ],
            }"#,
        )
        .unwrap();
        let tasks = find(root);
        let labels: Vec<&str> = tasks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Docs",
                "cargo build",
                "cargo test",
                "cargo run",
                "cargo clippy",
                "pnpm run dev",
                "pnpm run build",
                "make all",
                "make build",
                "just lint"
            ]
        );
        assert_eq!(tasks[0].command, "mdbook build 'my book'");
        assert_eq!(tasks[0].folder, root.join("docs"));
        assert_eq!(tasks[6].folder, root.join("web"));
        assert_eq!(default_build(&tasks).unwrap().label, "Docs");
    }
}
