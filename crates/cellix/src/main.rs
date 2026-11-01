use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cellix::Options;

fn workspace_root(dir: Option<&str>) -> std::io::Result<PathBuf> {
    match dir {
        Some(dir) => PathBuf::from(dir).canonicalize(),
        None => std::env::current_dir()?.canonicalize(),
    }
}

/// `cellix comments [--take] [dir]` and `cellix open <file> [dir]` talk to a running Cellix.
fn client(request: &str, dir: Option<&str>) -> ExitCode {
    let reply = workspace_root(dir).and_then(|root| cellix_agent::request(&root, request));
    match reply {
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cellix: no running Cellix for this workspace: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cellix hook <kind>` is what the Claude Code plugin calls. It always succeeds, because a
/// failing hook would get in the agent's way.
fn hook(kind: &str) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let Some(output) = cellix_agent::hook::handle(kind, &input) {
        println!("{output}");
    }
    ExitCode::SUCCESS
}

/// `cellix checkpoint [label]` marks the start of an agent's turn. It needs no running
/// Cellix, so a hook can call it whenever the agent is given a prompt.
fn checkpoint(label: &str) -> ExitCode {
    let label = if label.is_empty() {
        "Checkpoint"
    } else {
        label
    };
    let saved = workspace_root(None)
        .ok()
        .and_then(|root| cellix_review::checkpoint(&root, label));
    match saved {
        Some(turn) => {
            println!("{}", turn.number);
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("cellix: not in a git repository");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["--version" | "-V"] => {
            println!("cellix {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        ["checkpoint", label @ ..] => return checkpoint(&label.join(" ")),
        ["hook", kind] => return hook(kind),
        ["comments", "--take", rest @ ..] => return client("take", rest.first().copied()),
        ["comments", rest @ ..] => return client("comments", rest.first().copied()),
        ["open", file, rest @ ..] => {
            let path = std::path::absolute(file).unwrap_or_else(|_| PathBuf::from(file));
            return client(&format!("open {}", path.display()), rest.first().copied());
        }
        _ => {}
    }
    let mut options = Options::default();
    for word in words {
        match word {
            "--wait" => options.wait = true,
            path => options.paths.push(PathBuf::from(path)),
        }
    }
    match cellix::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cellix: {e}");
            ExitCode::FAILURE
        }
    }
}
