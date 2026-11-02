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

/// `cellix setup` unpacks the Claude Code plugin and says how to load it. It leaves Claude
/// Code's settings alone, because those are the user's: `--install` is how they ask.
fn setup(install: bool) -> ExitCode {
    if install {
        return match cellix::integrations::install_claude(|name| std::env::var(name).ok()) {
            Ok(()) => {
                println!("Installed. Claude Code now opens Cellix with no flags.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("cellix: could not install: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let Some(folder) = cellix::plugin::install() else {
        eprintln!("cellix: could not write the Claude Code plugin");
        return ExitCode::FAILURE;
    };
    println!("Plugin written to {}", folder.display());
    println!();
    println!("Install it for every Claude Code session, with no flags:");
    println!("  cellix setup --install");
    println!();
    println!("Or try it in one session:");
    println!("  claude --ide --plugin-dir {}", folder.display());
    ExitCode::SUCCESS
}

/// `cellix hook <kind>` is what the Claude Code plugin calls. It always succeeds, because a
/// failing hook would get in the agent's way.
fn hook(agent: &str, kind: &str) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let Some(output) = cellix_agent::hook::handle_for(agent, kind, &input) {
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
        ["hook", kind] => return hook("Claude Code", kind),
        ["hook", "--agent", agent, kind] => return hook(agent, kind),
        ["pane"] => {
            return match std::env::current_dir().and_then(|d| cellix::pane::run(&d.canonicalize()?))
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("cellix: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        ["setup"] => return setup(false),
        ["setup", "--install"] => return setup(true),
        ["setup", "--remove"] => {
            return match cellix::integrations::remove_claude(|name| std::env::var(name).ok()) {
                Ok(()) => {
                    println!("Removed. Claude Code no longer loads Cellix.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("cellix: could not remove: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        ["pane-send", input] => return client(&format!("pane {input}"), None),
        ["comments", "--take", rest @ ..] => return client("take", rest.first().copied()),
        ["comments", rest @ ..] => return client("comments", rest.first().copied()),
        ["threads"] => return client("threads", None),
        ["reply", id, text @ ..] if !text.is_empty() => {
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let text = text.join(" ").replace('\n', "\\n");
            return client(&format!("reply {id} {author}\t{text}"), None);
        }
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
