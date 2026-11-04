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
    match ask(request, dir) {
        Ok(reply) if reply.starts_with("error:") => {
            eprintln!("cellix: {}", reply.trim_start_matches("error: "));
            ExitCode::FAILURE
        }
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

/// Asks the Cellix serving this folder. From a subfolder, that's the repository's root.
fn ask(request: &str, dir: Option<&str>) -> std::io::Result<String> {
    let root = workspace_root(dir)?;
    let mut tried = cellix_agent::hook::workspaces(&root).into_iter();
    let first = tried.next().unwrap_or(root);
    tried.fold(cellix_agent::request(&first, request), |answer, next| {
        answer.or_else(|_| cellix_agent::request(&next, request))
    })
}

const USAGE: &str = "\
Usage: cellix [--wait] [files]

  cellix threads            The comments on this project, as JSON
  cellix reply <id> <text>  Answers a comment, beside the document
  cellix reply <id> --suggest <text>
                            Offers a rewrite of the words it's about, to apply
  cellix suggest <file> <line> <kind> <quote> <text>
                            Suggests a comment the reviewer can keep or dismiss
  cellix plan <file>        Holds a plan for review, and prints the verdict
  cellix comments [--take]  Review comments, as JSON
  cellix open <file>        Opens a file in the running Cellix
  cellix setup [claude|codex|gemini]  Connects an agent
  cellix setup --remove     Disconnects Claude Code
";

/// `cellix plan <file>` holds a plan until the reviewer decides. It exits 0 on approval,
/// printing the plan if it was edited, and 1 when it's sent back, with what to change.
fn plan(file: &str) -> ExitCode {
    let path = std::path::absolute(file).unwrap_or_else(|_| PathBuf::from(file));
    let reply = match ask(&format!("plan {}", path.display()), None) {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("cellix: no running Cellix for this workspace: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (verdict, text) = reply.split_once(' ').unwrap_or((&reply, "null"));
    let text: Option<String> = serde_json::from_str(text).unwrap_or(None);
    match verdict {
        "approve" => {
            println!("Approved.");
            if let Some(text) = text {
                println!("The reviewer edited the plan to read:\n{text}");
            }
            ExitCode::SUCCESS
        }
        "send-back" => {
            println!("Sent back: {}", text.unwrap_or_default());
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("cellix: the plan wasn't reviewed");
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

/// `cellix setup codex` and `gemini` do what their Install buttons in Settings do.
fn setup_agent(agent: &str) -> ExitCode {
    let var = |name: &str| std::env::var(name).ok();
    let (done, next) = if agent == "codex" {
        (
            cellix::integrations::install_codex(var),
            "Run /hooks once in Codex to trust its hooks.",
        )
    } else {
        (
            cellix::integrations::install_gemini(var),
            "Restart Gemini CLI to load it.",
        )
    };
    match done {
        Ok(()) => {
            println!("Installed. {next}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cellix: could not install: {e}");
            ExitCode::FAILURE
        }
    }
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
    // Settings and the agents' folders are found under HOME, which Windows sets only inside
    // Git Bash. Elsewhere it's the profile folder, which is where the agents keep theirs.
    #[cfg(windows)]
    if std::env::var_os("HOME").is_none()
        && let Some(profile) = std::env::var_os("USERPROFILE")
    {
        // SAFETY: the first thing main does, before any other thread exists.
        unsafe { std::env::set_var("HOME", profile) };
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["--version" | "-V"] => {
            println!("cellix {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        ["session-host", socket] => {
            return match cellix::hold::serve(std::path::Path::new(socket)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("cellix: {e}");
                    ExitCode::FAILURE
                }
            };
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
        ["setup", "--install" | "claude"] => return setup(true),
        ["setup", agent @ ("codex" | "gemini")] => return setup_agent(agent),
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
        ["--help" | "-h" | "help"] => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        ["threads"] => return client("threads", None),
        ["plan", file] => return plan(file),
        ["reply", id, "--suggest", text @ ..] if !text.is_empty() => {
            // A rewrite of the words the comment is about, which the reviewer can apply.
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let text = format!("Rewrite: {}", text.join(" ")).replace('\n', "\\n");
            return client(&format!("reply {id} {author}\t{text}"), None);
        }
        ["reply", id, text @ ..] if !text.is_empty() => {
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let text = text.join(" ").replace('\n', "\\n");
            return client(&format!("reply {id} {author}\t{text}"), None);
        }
        ["suggest", file, line, kind, quote, text @ ..] if !text.is_empty() => {
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let json = serde_json::json!({
                "file": file,
                "line": line.parse::<usize>().unwrap_or(1),
                "kind": kind,
                "quote": quote,
                "text": text.join(" "),
                "author": author,
            });
            return client(&format!("suggest {json}"), None);
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
