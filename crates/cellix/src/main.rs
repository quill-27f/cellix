use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cellix::Options;

fn workspace_root(dir: Option<&str>) -> std::io::Result<PathBuf> {
    match dir {
        Some(dir) => dunce::canonicalize(dir),
        None => dunce::canonicalize(std::env::current_dir()?),
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

/// `cellix agent …`: agents starting, messaging and waiting for each other, through the
/// Cellix that has this folder open. Prints what it's told, and ends with its code.
fn agent(args: &[&str]) -> ExitCode {
    use cellix_agent::team::{self, Answer, Asked, Caller, code};
    use std::io::IsTerminal;
    match args {
        ["--skill"] => {
            print!("{}", cellix::plugin::agent_guide());
            return ExitCode::SUCCESS;
        }
        [] | ["--help" | "-h" | "help"] => {
            print!("{}", team::usage());
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let stdin = || {
        let mut text = String::new();
        let _ = std::io::stdin().read_to_string(&mut text);
        text
    };
    let call = match team::parse(args, &cwd, stdin) {
        Ok(call) => call,
        Err(e) => {
            eprintln!("cellix agent: {e}");
            return ExitCode::from(code::USAGE);
        }
    };
    let from = Caller::here(
        |name| std::env::var(name).ok(),
        std::io::stdin().is_terminal(),
    );
    let asked = Asked { from, call };
    let line = format!(
        "agent {}",
        serde_json::to_string(&asked).unwrap_or_default()
    );
    let reply = match ask(&line, None) {
        Ok(reply) => reply,
        Err(_) => {
            eprintln!(
                "cellix agent: no Cellix has this folder open. Open it in Cellix (run cellix \
                 here), then try again."
            );
            return ExitCode::FAILURE;
        }
    };
    let answer: Answer = match serde_json::from_str(&reply) {
        Ok(answer) => answer,
        Err(_) => {
            // A Cellix from before `cellix agent` says it doesn't know the request.
            eprintln!(
                "cellix agent: the Cellix that has this folder open doesn't know this command; \
                 quit and start it again to update it"
            );
            return ExitCode::FAILURE;
        }
    };
    match &answer.out {
        serde_json::Value::Null => {}
        serde_json::Value::String(text) => println!("{text}"),
        out => println!("{out}"),
    }
    if !answer.error.is_empty() {
        eprintln!("cellix agent: {}", answer.error);
    }
    ExitCode::from(answer.code)
}

const USAGE: &str = "\
Usage: cellix [--wait] [files]

  cellix ls                 What's running in Cellix on this machine, by folder

  cellix agent run [--name N] [--cwd D] <claude|codex|gemini|command> [task]
                            Starts an agent in the side pane and types its task in
  cellix agent send <name> <text> [--wait] [--force]
                            Messages an agent, once the person isn't typing there
  cellix agent read <name>  What an agent last said
  cellix agent wait <name>  Until it stops working; --children all|any for your own
  cellix agent ls           The agents here, as JSON
  cellix agent stop <name>  Ends an agent
  cellix agent --skill      How agents use these, for an agent to read

  cellix threads            The comments on this project, as JSON
  cellix reply <id> <text>  Answers a comment, beside the document
  cellix reply <id> --suggest <text>
                            Offers a rewrite of the words it's about, to accept
  cellix reply <id> --done <text>
                            Says you did what it asks, and what: the reviewer resolves it
  cellix suggest <file> <line> <kind> <quote> <text>
                            Suggests a comment the reviewer can keep or dismiss
  cellix plan <file>        Holds a plan for review, and prints the verdict
  cellix comments [--take]  Review comments, as JSON
  cellix comments --waiting Everything no agent has had yet, a document's comments too
  cellix open <file>        Opens a file in the running Cellix
  cellix review <link>      Reviews a doc that lives elsewhere, read through Claude Code
  cellix setup [claude|codex|gemini|opencode|copilot|cursor|kiro]
                            Connects an agent
  cellix setup --remove     Disconnects Claude Code
  cellix setup <opencode|copilot|cursor|kiro> --remove
                            Disconnects that agent
";

/// `cellix ls`: each folder with something running in Cellix, stopped with the machine and
/// waiting to be resumed, or open as a workspace, in a Cellix that's running or the last one
/// that was. Cellix started at home offers the same list, less what another Cellix has open.
fn list() -> ExitCode {
    let found = cellix::hold::projects_with_activity();
    if found.is_empty() {
        println!("Nothing is running in Cellix on this machine.");
        return ExitCode::SUCCESS;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let rows: Vec<(String, String, String)> = found
        .iter()
        .map(|(root, activity)| {
            let name = root.file_name().map_or_else(
                || root.display().to_string(),
                |n| n.to_string_lossy().into(),
            );
            let mut what = activity.running.join(", ");
            if activity.running.is_empty() && activity.stopped.is_empty() {
                what.push_str("open, nothing running");
            }
            if !activity.stopped.is_empty() {
                if !what.is_empty() {
                    what.push_str("; ");
                }
                what.push_str(&format!("stopped: {}", activity.stopped.join(", ")));
            }
            let path = match home.as_deref().and_then(|h| root.strip_prefix(h).ok()) {
                Some(rest) => format!("~/{}", rest.display()),
                None => root.display().to_string(),
            };
            (name, what, path)
        })
        .collect();
    let name_width = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let what_width = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    for (name, what, path) in rows {
        println!("{name:<name_width$}  {what:<what_width$}  {path}");
    }
    println!("\nRun cellix in one of these folders to pick it up, or cellix at home to choose.");
    ExitCode::SUCCESS
}

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
                if cellix::integrations::on_path("tmux", |name| std::env::var(name).ok()) {
                    println!(
                        "tmux's config now has allow-passthrough and focus-events on, which Claude Code needs to show pictures inside tmux."
                    );
                }
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

/// `cellix setup codex` and the others do what their Install buttons in Settings do.
fn setup_agent(agent: &str) -> ExitCode {
    use cellix::integrations;
    let var = |name: &str| std::env::var(name).ok();
    let (done, next) = match agent {
        "codex" => (
            integrations::install_codex(var),
            "Run /hooks once in Codex to trust its hooks.",
        ),
        "opencode" => (
            integrations::install_opencode(var),
            "OpenCode loads it as it starts.",
        ),
        "copilot" => (
            integrations::install_copilot(var),
            "Copilot CLI loads it as it starts.",
        ),
        "cursor" => (
            integrations::install_cursor(var),
            "Cursor's agent loads it as it starts.",
        ),
        "kiro" => return setup_kiro(),
        _ => (
            integrations::install_gemini(var),
            "Restart Gemini CLI to load it.",
        ),
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

/// `cellix setup kiro` writes Kiro CLI's hooks as the Kiro CLI on the PATH runs them, and says
/// what one too old for them needs, writing nowhere else.
fn setup_kiro() -> ExitCode {
    use cellix::integrations;
    let var = |name: &str| std::env::var(name).ok();
    let version = integrations::kiro_version(var);
    match integrations::install_kiro(var, version) {
        Ok(()) => {
            println!("Installed. {}", integrations::kiro_note(version));
            if version.is_none_or(|(major, _)| major >= 3) {
                println!(
                    "If you've chosen its V2 chat, this turns V3 back on: kiro-cli settings chat.agentEngine v3"
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cellix: could not install: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cellix setup opencode --remove` and the others take back exactly what setup added.
fn remove_agent(agent: &str) -> ExitCode {
    use cellix::integrations;
    let var = |name: &str| std::env::var(name).ok();
    let done = match agent {
        "opencode" => integrations::remove_opencode(var),
        "copilot" => integrations::remove_copilot(var),
        "kiro" => integrations::remove_kiro(var),
        _ => integrations::remove_cursor(var),
    };
    match done {
        Ok(()) => {
            println!("Removed.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cellix: could not remove: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cellix hook <kind>` is what the Claude Code plugin calls. It always succeeds, because a
/// failing hook would get in the agent's way.
fn hook(agent: &str, kind: &str) -> ExitCode {
    // An agent run to answer a side question: no hook of Cellix's does anything for it.
    if cellix_agent::hook::side_question() {
        return ExitCode::SUCCESS;
    }
    if agent == "Claude Code" && kind == "session-start" {
        cellix::plugin::refresh();
        // Too late for this session, which has read tmux's options already, but every
        // Claude Code started after in this tmux shows Cellix's pictures.
        let _ = cellix::pane::prepare_tmux(|name| std::env::var(name).ok());
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    // Kiro CLI hands a command hook the prompt in `USER_PROMPT`.
    if agent == "Kiro" && kind == "prompt" {
        let prompt = std::env::var("USER_PROMPT").ok();
        input = cellix_agent::hook::with_prompt(&input, prompt.as_deref());
    }
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
    // The prompt hook passes the prompt, to log with the checkpoint for the task view.
    let prompt: Option<cellix_review::tasks::Event> =
        std::env::var(cellix_review::tasks::EVENT_VAR)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());
    let log = std::env::var_os(cellix_review::tasks::ROOT_VAR).map(std::path::PathBuf::from);
    let prompt = log.as_deref().zip(prompt);
    let saved = workspace_root(None)
        .ok()
        .and_then(|root| cellix_review::tasks::checkpoint_prompt(&root, label, prompt));
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
        ["agent", rest @ ..] => return agent(rest),
        ["ls"] => return list(),
        ["hook", kind] => return hook("Claude Code", kind),
        ["hook", "--agent", agent, kind] => return hook(agent, kind),
        // Claude Code run for a side question loads the mod too, which would start a pane.
        ["pane"] if cellix_agent::hook::side_question() => return ExitCode::SUCCESS,
        ["pane"] => {
            return match std::env::current_dir()
                .and_then(|d| cellix::pane::run(&dunce::canonicalize(d)?))
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
        [
            "setup",
            agent @ ("codex" | "gemini" | "opencode" | "copilot" | "cursor" | "kiro"),
        ] => {
            return setup_agent(agent);
        }
        [
            "setup",
            agent @ ("opencode" | "copilot" | "cursor" | "kiro"),
            "--remove",
        ] => {
            return remove_agent(agent);
        }
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
        ["comments", "--waiting", rest @ ..] => return client("waiting", rest.first().copied()),
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
            let text = cellix_agent::escape(&format!("Rewrite: {}", text.join(" ")));
            return client(&format!("reply {id} {author}\t{text}"), None);
        }
        ["reply", id, "--done", text @ ..] => {
            // Done as asked: the card says so, and the reviewer resolves it or says it isn't.
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let text = cellix_agent::escape(&cellix_review::threads::addressed(&text.join(" ")));
            return client(&format!("reply {id} {author}\t{text}"), None);
        }
        ["reply", id, text @ ..] if !text.is_empty() => {
            let author = std::env::var("CELLIX_AGENT").unwrap_or_else(|_| "Agent".to_string());
            let text = cellix_agent::escape(&text.join(" "));
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
        // The running Cellix has its agent read the doc, and opens it when it's read.
        ["review", link, rest @ ..] => {
            return client(&format!("review {link}"), rest.first().copied());
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
