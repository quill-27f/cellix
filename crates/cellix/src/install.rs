//! How a language server gets onto this machine, and where Cellix finds it after.
//!
//! Each server lists the ways it can be installed, in the order they're tried, and its Install
//! button runs the first this machine can: Homebrew where it's on the PATH, npm into the
//! person's own folder where npm's own isn't theirs to write, the project's build for this
//! system fetched into `~/.local/bin`, or a tool's own installer. None of them asks for a
//! password: an install runs with no terminal, so sudo couldn't ask. Where only a system
//! package or a tool that isn't here can give a server, its row says what's needed and offers
//! the command to copy, rather than a button that can only fail.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::settings::Server;

/// One way to install a server.
#[derive(Clone, Copy, Debug)]
pub enum Way {
    /// `brew install` with these words, where Homebrew is on the PATH.
    Brew(&'static str),
    /// These npm packages, installed for everyone on the machine: into npm's own folder where
    /// the person can write it, and into `~/.local` where they can't, whose `bin` Cellix searches.
    Npm(&'static str),
    /// A tool's own command, run as it's written where its first word is a program here, and
    /// what to call the tool where it isn't.
    Tool(&'static str, &'static str),
    /// A command only a Mac has.
    Mac(&'static str),
    /// The project's own build for this system and processor, fetched into `~/.local/bin`.
    Release(Release),
    /// A package only the system's package manager gives, by its name in each manager.
    System(&'static [(&'static str, &'static str)]),
    /// What has it, where nothing here can install it: words for the row.
    Needs(&'static str),
}

/// A project that publishes its server built for each system, and how the build is packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Marksman,
    RustAnalyzer,
    Taplo,
    Texlab,
    Zls,
    TerraformLs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Packed {
    /// The program itself.
    Bare,
    /// The program, gzipped.
    Gz,
    /// A tar file holding the program at its top, gzipped or in xz.
    TarGz,
    TarXz,
    /// A zip file holding the program at its top.
    Zip,
}

/// HashiCorp's list of its latest release of terraform-ls, which names each build's file.
const HASHICORP_LATEST: &str = "https://api.releases.hashicorp.com/v1/releases/terraform-ls/latest";

impl Release {
    fn program(self) -> &'static str {
        match self {
            Release::Marksman => "marksman",
            Release::RustAnalyzer => "rust-analyzer",
            Release::Taplo => "taplo",
            Release::Texlab => "texlab",
            Release::Zls => "zls",
            Release::TerraformLs => "terraform-ls",
        }
    }

    fn packed(self) -> Packed {
        match self {
            Release::Marksman => Packed::Bare,
            Release::RustAnalyzer | Release::Taplo => Packed::Gz,
            Release::Texlab => Packed::TarGz,
            Release::Zls => Packed::TarXz,
            Release::TerraformLs => Packed::Zip,
        }
    }

    /// The name of the build's file for a system and processor, as Rust names them, if the
    /// project makes one.
    fn asset(self, os: &str, arch: &str) -> Option<String> {
        let mac = match os {
            "macos" => true,
            "linux" => false,
            _ => return None,
        };
        if !matches!(arch, "x86_64" | "aarch64") {
            return None;
        }
        let arm = arch == "aarch64";
        Some(match self {
            // One build for both of a Mac's processors.
            Release::Marksman if mac => "marksman-macos".to_string(),
            Release::Marksman if arm => "marksman-linux-arm64".to_string(),
            Release::Marksman => "marksman-linux-x64".to_string(),
            Release::RustAnalyzer if mac => format!("rust-analyzer-{arch}-apple-darwin.gz"),
            Release::RustAnalyzer => format!("rust-analyzer-{arch}-unknown-linux-gnu.gz"),
            // Its "full" builds are the ones with the language server in them.
            Release::Taplo if mac => format!("taplo-full-darwin-{arch}.gz"),
            Release::Taplo => format!("taplo-full-linux-{arch}.gz"),
            Release::Texlab if mac => format!("texlab-{arch}-macos.tar.gz"),
            Release::Texlab => format!("texlab-{arch}-linux.tar.gz"),
            Release::Zls if mac => format!("zls-{arch}-macos.tar.xz"),
            Release::Zls => format!("zls-{arch}-linux.tar.xz"),
            Release::TerraformLs => {
                let system = if mac { "darwin" } else { "linux" };
                let processor = if arm { "arm64" } else { "amd64" };
                format!("terraform-ls_{system}_{processor}.zip")
            }
        })
    }

    /// Who publishes it, as the row says where it's fetched from.
    fn from(self) -> &'static str {
        match self {
            Release::TerraformLs => "HashiCorp",
            _ => "GitHub",
        }
    }

    /// Where its latest build of that name is.
    fn url(self, asset: &str) -> String {
        let repository = match self {
            Release::Marksman => "artempyanykh/marksman",
            Release::RustAnalyzer => "rust-lang/rust-analyzer",
            Release::Taplo => "tamasfe/taplo",
            Release::Texlab => "latex-lsp/texlab",
            Release::Zls => "zigtools/zls",
            // Named by version, so found in HashiCorp's list as the install runs.
            Release::TerraformLs => return HASHICORP_LATEST.to_string(),
        };
        format!("https://github.com/{repository}/releases/latest/download/{asset}")
    }
}

/// What a server's Install does on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A command for `sh`, and how its row says it: "Runs" and the command, or for a build
    /// that's fetched, where from and where to.
    Run { shell: String, says: String },
    /// Nothing here can install it: what's needed, and a command for the person to run, where
    /// one would help.
    Needs { says: String, copy: Option<String> },
}

impl Plan {
    /// The words its row shows while it's pointed at.
    pub fn says(&self) -> &str {
        match self {
            Plan::Run { says, .. } | Plan::Needs { says, .. } => says,
        }
    }
}

/// What a machine has that installing turns on: its system and processor, the person's home,
/// the PATH Cellix was started with, and where Go, Cargo and npm have been told to put things.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    /// `macos` or `linux`, as Rust names systems.
    pub os: &'static str,
    /// `x86_64` or `aarch64`, as Rust names processors.
    pub arch: &'static str,
    pub home: Option<PathBuf>,
    pub path: OsString,
    pub gobin: Option<PathBuf>,
    pub gopath: Option<OsString>,
    pub cargo_home: Option<PathBuf>,
    /// npm's prefix where its own variable sets it.
    pub npm_prefix: Option<PathBuf>,
    /// Whether npm's global folder is the person's to write, where that's been settled rather
    /// than looked at on the disk.
    pub npm_writable: Option<bool>,
}

/// How a build is fetched: by curl, or by wget where there's no curl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetch {
    Curl,
    Wget,
}

impl Fetch {
    fn to_file(self, url: &str, file: &str) -> String {
        match self {
            Fetch::Curl => format!("curl -fsSL -o {file} {url}"),
            Fetch::Wget => format!("wget -q -O {file} {url}"),
        }
    }

    fn to_stdout(self, url: &str) -> String {
        match self {
            Fetch::Curl => format!("curl -fsSL {url}"),
            Fetch::Wget => format!("wget -q -O - {url}"),
        }
    }
}

/// The package managers whose packages a server can come from, by the program that runs
/// each, the name a server's `System` way knows it by, and what installs with it.
const MANAGERS: [(&str, &str, &str); 6] = [
    ("dnf", "dnf", "sudo dnf install"),
    ("yum", "dnf", "sudo yum install"),
    ("apt-get", "apt", "sudo apt install"),
    ("pacman", "pacman", "sudo pacman -S"),
    ("zypper", "zypper", "sudo zypper install"),
    ("apk", "apk", "sudo apk add"),
];

impl Machine {
    /// This machine, as Cellix was started on it.
    pub fn here() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            home: var("HOME").map(PathBuf::from),
            path: var("PATH").unwrap_or_default(),
            gobin: var("GOBIN").map(PathBuf::from),
            gopath: var("GOPATH"),
            cargo_home: var("CARGO_HOME").map(PathBuf::from),
            npm_prefix: var("npm_config_prefix")
                .or_else(|| var("NPM_CONFIG_PREFIX"))
                .map(PathBuf::from),
            npm_writable: None,
        }
    }

    /// The folders that installing without root puts programs in, which a PATH often lacks:
    /// where Cellix was started by a service, an IDE, or a shell whose profile doesn't add them.
    pub fn user_bins(&self) -> Vec<PathBuf> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        let go = self.gobin.clone().or_else(|| {
            let first = std::env::split_paths(self.gopath.as_ref()?).next()?;
            Some(first.join("bin"))
        });
        let cargo = self
            .cargo_home
            .clone()
            .unwrap_or_else(|| home.join(".cargo"));
        vec![
            home.join(".local/bin"),
            cargo.join("bin"),
            go.unwrap_or_else(|| home.join("go/bin")),
            home.join(".dotnet/tools"),
        ]
    }

    /// Where programs are looked for: the PATH, then those folders where it lacks them, and
    /// Homebrew's LLVM, which it keeps off the PATH on a Mac, clangd with it.
    fn dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&self.path)
            .filter(|d| !d.as_os_str().is_empty())
            .collect();
        for dir in self.user_bins() {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        let brew = find_in(&dirs, "brew");
        if let Some(prefix) = brew
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            let llvm = prefix.join("opt/llvm/bin");
            if !dirs.contains(&llvm) {
                dirs.push(llvm);
            }
        }
        dirs
    }

    /// The PATH an install runs with, and a server started from: the one Cellix has, with the
    /// folders above after it.
    pub fn search_path(&self) -> OsString {
        std::env::join_paths(self.dirs()).unwrap_or_else(|_| self.path.clone())
    }

    /// Where a program is, by its name or its path.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        if program.contains('/') || program.contains(std::path::MAIN_SEPARATOR) {
            let path = PathBuf::from(program);
            return path.is_file().then_some(path);
        }
        find_in(&self.dirs(), program)
    }

    fn has(&self, program: &str) -> bool {
        self.find(program).is_some()
    }

    fn fetch(&self) -> Option<Fetch> {
        if self.has("curl") {
            Some(Fetch::Curl)
        } else if self.has("wget") {
            Some(Fetch::Wget)
        } else {
            None
        }
    }

    /// The folder npm installs into with `-g`: as its variable or the person's `~/.npmrc` sets
    /// it, or else the one over the folder node is in, as npm itself works it out.
    fn npm_prefix(&self) -> Option<PathBuf> {
        if let Some(prefix) = &self.npm_prefix {
            return Some(prefix.clone());
        }
        if let Some(home) = &self.home
            && let Ok(rc) = fs::read_to_string(home.join(".npmrc"))
        {
            let set = rc.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "prefix").then(|| value.trim().to_string())
            });
            if let Some(set) = set.filter(|s| !s.is_empty()) {
                return Some(expand_home(&set, home));
            }
        }
        let node = self.find("node")?;
        Some(node.parent()?.parent()?.to_path_buf())
    }

    /// Whether `npm install -g` can write where it installs without root.
    fn npm_writable(&self) -> bool {
        if let Some(known) = self.npm_writable {
            return known;
        }
        let Some(prefix) = self.npm_prefix() else {
            return false;
        };
        let modules = [
            prefix.join("lib/node_modules"),
            prefix.join("lib"),
            prefix.clone(),
        ];
        let into = modules.into_iter().find(|dir| dir.is_dir());
        let bin = prefix.join("bin");
        into.is_some_and(|dir| writable(&dir)) && (!bin.exists() || writable(&bin))
    }

    /// What installs a system package with the manager this machine has, if it has one the
    /// package is known to.
    fn system_install(&self, names: &[(&str, &str)]) -> Option<String> {
        MANAGERS.iter().find_map(|(program, known_as, install)| {
            if !self.has(program) {
                return None;
            }
            let name = names.iter().find(|(manager, _)| manager == known_as)?.1;
            Some(format!("{install} {name}"))
        })
    }

    /// What a way runs here, if this machine can run it.
    fn run(&self, way: Way, program: &str) -> Option<Plan> {
        let plain = |command: String| Plan::Run {
            says: format!("Runs {command}"),
            shell: command,
        };
        match way {
            Way::Brew(words) => self
                .has("brew")
                .then(|| plain(format!("brew install {words}"))),
            Way::Npm(packages) if self.has("npm") => Some(if self.npm_writable() {
                plain(format!("npm install -g {packages}"))
            } else {
                Plan::Run {
                    shell: format!("npm install -g --prefix \"$HOME/.local\" {packages}"),
                    says: format!("Runs npm install -g --prefix ~/.local {packages}"),
                }
            }),
            Way::Tool(command, _) => {
                let tool = command.split_whitespace().next()?;
                self.has(tool).then(|| plain(command.to_string()))
            }
            Way::Mac(command) => (self.os == "macos").then(|| plain(command.to_string())),
            Way::Release(release) => {
                let asset = release.asset(self.os, self.arch)?;
                let fetch = self.fetch()?;
                let unpack = match release.packed() {
                    Packed::Bare => None,
                    Packed::Gz => Some("gunzip"),
                    Packed::TarGz => Some("tar"),
                    // A Mac's tar reads xz itself, and Linux's has xz do it.
                    Packed::TarXz if self.os == "macos" => Some("tar"),
                    Packed::TarXz => Some("xz"),
                    Packed::Zip => Some("unzip"),
                };
                if unpack.is_some_and(|tool| !self.has(tool)) {
                    return None;
                }
                let into = "~/.local/bin";
                let from = release.from();
                let says = format!("Fetches {asset} from {from} into {into}");
                let shell = release_script(release, &asset, fetch);
                debug_assert_eq!(release.program(), program);
                Some(Plan::Run { shell, says })
            }
            Way::Npm(_) | Way::System(_) | Way::Needs(_) => None,
        }
    }

    /// What a way needs that this machine lacks, in words for the row: nothing, where it
    /// doesn't apply to this system.
    fn lacks(&self, way: Way, program: &str) -> Option<String> {
        match way {
            Way::Brew(_) => Some("Homebrew".to_string()),
            Way::Npm(_) => Some("npm".to_string()),
            Way::Tool(_, tool) => Some(tool.to_string()),
            Way::Mac(_) => None,
            Way::Release(release) => {
                let asset = release.asset(self.os, self.arch)?;
                if self.fetch().is_none() {
                    return Some("curl".to_string());
                }
                let unpack = match release.packed() {
                    Packed::Gz => "gunzip",
                    Packed::Zip => "unzip",
                    Packed::TarXz if self.os != "macos" => "xz",
                    _ => "tar",
                };
                Some(format!("{unpack}, to unpack {asset}"))
            }
            Way::System(names) if self.os == "linux" => Some(
                self.system_install(names)
                    .unwrap_or_else(|| format!("your system's {program} package")),
            ),
            Way::System(_) => None,
            Way::Needs(words) => Some(words.to_string()),
        }
    }

    /// A command a way gives the person to run themselves.
    fn copy(&self, way: Way) -> Option<String> {
        match way {
            Way::Brew(words) => Some(format!("brew install {words}")),
            Way::Npm(packages) => Some(format!("npm install -g {packages}")),
            Way::Tool(command, _) => Some(command.to_string()),
            Way::System(names) => self.system_install(names),
            Way::Mac(_) | Way::Release(_) | Way::Needs(_) => None,
        }
    }
}

/// What a server's Install does on this machine: the first of its ways the machine can run,
/// or what it would need.
pub fn plan(server: &Server, machine: &Machine) -> Plan {
    let program = server.command[0];
    if let Some(plan) = server
        .ways
        .iter()
        .find_map(|way| machine.run(*way, program))
    {
        return plan;
    }
    let mut needs: Vec<String> = Vec::new();
    for way in server.ways {
        if let Some(lack) = machine.lacks(*way, program)
            && !needs.contains(&lack)
        {
            needs.push(lack);
        }
    }
    // The system's own package is what most people here would run, then the first way's.
    let system = server.ways.iter().find(|w| matches!(w, Way::System(_)));
    let copy = system
        .and_then(|way| machine.copy(*way))
        .or_else(|| server.ways.iter().find_map(|way| machine.copy(*way)));
    let says = if needs.is_empty() {
        format!("Nothing installs {program} on this system")
    } else {
        format!("Needs {}", needs.join(", or "))
    };
    Plan::Needs { says, copy }
}

/// The shell that fetches a release's build, unpacks it and puts the program in
/// `~/.local/bin`, through a folder of its own so a download that fails leaves nothing behind.
fn release_script(release: Release, asset: &str, fetch: Fetch) -> String {
    let program = release.program();
    let got = "\"$t/get\"";
    let fetching = if release == Release::TerraformLs {
        // HashiCorp names its files by version, so its list of the latest says which.
        let tail = asset.trim_start_matches("terraform-ls");
        let pattern = format!("https://releases.hashicorp.com/terraform-ls/[^\"]*{tail}");
        format!(
            "url=$({} | grep -o '{pattern}' | head -n 1); [ -n \"$url\" ] || {{ echo \"HashiCorp lists no {asset}\" >&2; exit 1; }}; {}",
            fetch.to_stdout(&release.url(asset)),
            fetch.to_file("\"$url\"", got),
        )
    } else {
        fetch.to_file(&release.url(asset), got)
    };
    let unpack = match release.packed() {
        Packed::Bare => format!("mv -f {got} \"$t/{program}\""),
        Packed::Gz => format!("gunzip -c {got} > \"$t/{program}\""),
        Packed::TarGz => format!("tar -xzf {got} -C \"$t\""),
        Packed::TarXz => format!("tar -xJf {got} -C \"$t\""),
        Packed::Zip => format!("unzip -o -q {got} -d \"$t\""),
    };
    format!(
        "set -e; d=\"$HOME/.local/bin\"; mkdir -p \"$d\"; t=$(mktemp -d); trap 'rm -rf \"$t\"' EXIT; \
         {fetching}; {unpack}; chmod +x \"$t/{program}\"; mv -f \"$t/{program}\" \"$d/{program}\""
    )
}

fn find_in(dirs: &[PathBuf], program: &str) -> Option<PathBuf> {
    // Windows runs `npm` as `npm.cmd`.
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ending| format!("{program}{ending}"))
            .collect()
    } else {
        vec![program.to_string()]
    };
    dirs.iter().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file())
    })
}

/// A path from a config file, with `~` and `$HOME` read as the home folder.
fn expand_home(path: &str, home: &Path) -> PathBuf {
    let rest = ["~/", "$HOME/", "${HOME}/"]
        .iter()
        .find_map(|start| path.strip_prefix(start));
    match rest {
        Some(rest) => home.join(rest),
        None if matches!(path, "~" | "$HOME" | "${HOME}") => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a C string that lives across the call, which only reads it.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
}

/// Runs an install's command, in a session of its own with no terminal and with the folders
/// installs use on its PATH, and says why it failed if it did.
pub fn run(shell: &str, program: &str, machine: &Machine) -> Result<(), String> {
    // Not a login shell, whose profile would swap the PATH Cellix was started with. By its
    // path: a PATH given to a command is where its own program is looked for too.
    let mut command = std::process::Command::new(if cfg!(unix) { "/bin/sh" } else { "sh" });
    command
        .args(["-c", shell])
        .env("PATH", machine.search_path())
        .stdin(std::process::Stdio::null());
    if let Some(home) = &machine.home {
        command.env("HOME", home);
    }
    // In a session of its own, with no terminal. An installer that asks on /dev/tty, as
    // terminal-browser's `setup` does, would otherwise ask on Cellix's own screen, where
    // Enter arrives as \r and never answers it, and the install would never end. With no
    // terminal it takes its default answer, and sudo fails instead of waiting for a password.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe, and this runs in the child before it execs.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    match command.output() {
        Ok(done) if done.status.success() => Ok(()),
        Ok(done) => {
            let said = format!(
                "{}\n{}",
                String::from_utf8_lossy(&done.stdout),
                String::from_utf8_lossy(&done.stderr)
            );
            Err(explain(program, shell, done.status.code(), &said))
        }
        Err(e) => Err(format!("Couldn't install {program}: {e}")),
    }
}

/// Words with their first letter a capital, to stand as a sentence of their own.
pub fn capitalized(words: &str) -> String {
    let mut letters = words.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// Why an install failed, in words the person can act on, from what it printed.
pub fn explain(program: &str, shell: &str, code: Option<i32>, said: &str) -> String {
    let lower = said.to_lowercase();
    let has = |words: &str| lower.contains(words);
    let last = said.lines().map(str::trim).rfind(|line| !line.is_empty());
    let why = if has("sudo") && (has("password") || has("terminal is required")) {
        "it asks for sudo's password, which needs a terminal. Run it in one".to_string()
    } else if has("eacces") || has("permission denied") || has("operation not permitted") {
        "it isn't allowed to write where it installs. Run it in a terminal".to_string()
    } else if has("could not resolve")
        || has("unable to resolve")
        || has("enotfound")
        || has("eai_again")
        || has("network is unreachable")
        || has("failed to connect")
    {
        "no connection to the internet".to_string()
    } else if has("404") || has("hashicorp lists no") {
        "the download isn't there".to_string()
    } else if code == Some(127) || has("command not found") || has(": not found") {
        let first = shell.split_whitespace().next().unwrap_or(shell);
        format!("{first} isn't on the PATH")
    } else {
        last.unwrap_or("the install failed").to_string()
    };
    format!("Couldn't install {program}: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SERVERS;

    fn server(language: &str) -> &'static Server {
        SERVERS
            .iter()
            .find(|s| s.languages.contains(&language))
            .unwrap()
    }

    /// A machine with these programs and nothing else, as stand-ins in a folder of its own.
    fn machine(os: &'static str, programs: &[&str]) -> (tempfile::TempDir, Machine) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for program in programs {
            fs::write(bin.join(program), "#!/bin/sh\n").unwrap();
        }
        let machine = Machine {
            os,
            arch: "x86_64",
            home: Some(dir.path().join("home")),
            path: bin.into_os_string(),
            ..Machine::default()
        };
        (dir, machine)
    }

    fn says(plan: &Plan) -> &str {
        plan.says()
    }

    #[test]
    fn each_machine_gets_a_way_it_can_run() {
        // Homebrew, where it's on the PATH, as on a Mac or with Linuxbrew.
        let (_dir, mac) = machine("macos", &["brew", "curl"]);
        let (_d, linuxbrew) = machine("linux", &["brew", "curl"]);
        for machine in [&mac, &linuxbrew] {
            let marksman = plan(server("Markdown"), machine);
            assert_eq!(says(&marksman), "Runs brew install marksman");
        }
        // Without it, the project's own build for the system and processor.
        let (_d, linux) = machine("linux", &["curl", "gunzip", "tar", "xz"]);
        let marksman = plan(server("Markdown"), &linux);
        assert_eq!(
            says(&marksman),
            "Fetches marksman-linux-x64 from GitHub into ~/.local/bin"
        );
        let Plan::Run { shell, .. } = &marksman else {
            panic!("{marksman:?}")
        };
        assert!(
            shell.contains(
                "curl -fsSL -o \"$t/get\" https://github.com/artempyanykh/marksman/releases/latest/download/marksman-linux-x64"
            ),
            "{shell}"
        );
        assert!(shell.contains("\"$HOME/.local/bin\""), "{shell}");
        let arm = Machine {
            arch: "aarch64",
            ..linux.clone()
        };
        assert!(says(&plan(server("Markdown"), &arm)).contains("marksman-linux-arm64"));
        let texlab = plan(server("LaTeX"), &arm);
        assert!(
            says(&texlab).contains("texlab-aarch64-linux.tar.gz"),
            "{texlab:?}"
        );
        let taplo = plan(server("TOML"), &linux);
        assert!(
            says(&taplo).contains("taplo-full-linux-x86_64.gz"),
            "{taplo:?}"
        );
        // wget where there's no curl.
        let (_d, wget) = machine("linux", &["wget"]);
        let Plan::Run { shell, .. } = plan(server("Markdown"), &wget) else {
            panic!("wget fetches it")
        };
        assert!(shell.contains("wget -q -O \"$t/get\" https://"), "{shell}");
        // A Mac without Homebrew fetches the Mac's build.
        let (_d, bare_mac) = machine("macos", &["curl", "tar"]);
        assert!(says(&plan(server("Markdown"), &bare_mac)).contains("marksman-macos"));
        // The tools' own installers, where they're here.
        let (_d, tools) = machine("linux", &["rustup", "go", "dotnet", "gem"]);
        for (language, command) in [
            ("Rust", "rustup component add rust-analyzer"),
            ("Go", "go install golang.org/x/tools/gopls@latest"),
            ("C#", "dotnet tool install --global csharp-ls"),
            (
                "Ruby",
                "gem install --user-install --bindir ~/.local/bin ruby-lsp",
            ),
        ] {
            let plan = plan(server(language), &tools);
            assert_eq!(says(&plan), format!("Runs {command}"));
            assert_eq!(
                plan,
                Plan::Run {
                    shell: command.to_string(),
                    says: format!("Runs {command}"),
                }
            );
        }
    }

    #[test]
    fn npm_installs_into_the_persons_own_folder_where_its_own_is_not_theirs() {
        let (_dir, mut linux) = machine("linux", &["npm"]);
        linux.npm_writable = Some(true);
        assert_eq!(
            says(&plan(server("Python"), &linux)),
            "Runs npm install -g pyright"
        );
        linux.npm_writable = Some(false);
        let pyright = plan(server("Python"), &linux);
        assert_eq!(
            says(&pyright),
            "Runs npm install -g --prefix ~/.local pyright"
        );
        let Plan::Run { shell, .. } = pyright else {
            panic!()
        };
        assert_eq!(shell, "npm install -g --prefix \"$HOME/.local\" pyright");
    }

    /// npm's folder is where npm would put it: its variable, the person's .npmrc, or over node.
    #[test]
    fn npms_folder_is_found_as_npm_finds_it() {
        let (dir, mut linux) = machine("linux", &["npm"]);
        let system = dir.path().join("usr");
        fs::create_dir_all(system.join("bin")).unwrap();
        fs::write(system.join("bin/node"), "").unwrap();
        let mut path = std::env::split_paths(&linux.path).collect::<Vec<_>>();
        path.push(system.join("bin"));
        linux.path = std::env::join_paths(path).unwrap();
        assert_eq!(linux.npm_prefix(), Some(system.clone()));
        let home = linux.home.clone().unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(".npmrc"), "color=false\nprefix = ~/.npm-global\n").unwrap();
        assert_eq!(linux.npm_prefix(), Some(home.join(".npm-global")));
        linux.npm_prefix = Some(dir.path().join("set"));
        assert_eq!(linux.npm_prefix(), Some(dir.path().join("set")));
        // One the person owns can be written; one with no folder at all can't.
        linux.npm_prefix = Some(dir.path().to_path_buf());
        assert!(linux.npm_writable());
        linux.npm_prefix = None;
        fs::remove_file(home.join(".npmrc")).unwrap();
        linux.path = dir.path().join("bin").into_os_string();
        assert!(!linux.npm_writable(), "no node, so no folder");
    }

    #[test]
    fn what_only_a_system_package_or_a_missing_tool_gives_says_so_with_a_command() {
        // clangd on Amazon Linux, which has dnf and no Homebrew.
        let (_dir, amazon) = machine("linux", &["dnf", "curl"]);
        let clangd = plan(server("C"), &amazon);
        assert_eq!(
            clangd,
            Plan::Needs {
                says: "Needs Homebrew, or sudo dnf install clang-tools-extra".to_string(),
                copy: Some("sudo dnf install clang-tools-extra".to_string()),
            }
        );
        let (_dir, debian) = machine("linux", &["apt-get"]);
        assert_eq!(
            plan(server("C"), &debian),
            Plan::Needs {
                says: "Needs Homebrew, or sudo apt install clangd".to_string(),
                copy: Some("sudo apt install clangd".to_string()),
            }
        );
        // jdtls has only Homebrew.
        let jdtls = plan(server("Java"), &amazon);
        assert_eq!(
            jdtls,
            Plan::Needs {
                says: "Needs Homebrew".to_string(),
                copy: Some("brew install jdtls".to_string()),
            }
        );
        // An npm package with no npm, and a build with nothing to fetch it.
        let (_dir, bare) = machine("linux", &[]);
        assert_eq!(
            plan(server("Python"), &bare),
            Plan::Needs {
                says: "Needs npm, or Homebrew".to_string(),
                copy: Some("npm install -g pyright".to_string()),
            }
        );
        assert_eq!(
            says(&plan(server("Markdown"), &bare)),
            "Needs Homebrew, or curl"
        );
        // Swift comes with its toolchain off a Mac, and xcode-select on one.
        let swift = plan(server("Swift"), &bare);
        assert_eq!(
            swift,
            Plan::Needs {
                says: "Needs Swift, from swift.org".to_string(),
                copy: None,
            }
        );
        let (_dir, mac) = machine("macos", &[]);
        assert_eq!(
            says(&plan(server("Swift"), &mac)),
            "Runs xcode-select --install"
        );
        // Every server says something on every machine, and none runs a sudo.
        for machine in [&amazon, &debian, &bare, &mac] {
            for server in &SERVERS {
                let plan = plan(server, machine);
                assert!(!plan.says().is_empty());
                if let Plan::Run { shell, .. } = &plan {
                    assert!(!shell.contains("sudo"), "{shell}");
                }
            }
        }
    }

    #[test]
    fn programs_are_found_in_the_folders_installs_use_when_the_path_lacks_them() {
        let (dir, mut linux) = machine("linux", &[]);
        let home = linux.home.clone().unwrap();
        for folder in [".local/bin", ".cargo/bin", "go/bin", ".dotnet/tools"] {
            fs::create_dir_all(home.join(folder)).unwrap();
        }
        fs::write(home.join(".local/bin/marksman"), "").unwrap();
        fs::write(home.join(".cargo/bin/rust-analyzer"), "").unwrap();
        fs::write(home.join("go/bin/gopls"), "").unwrap();
        fs::write(home.join(".dotnet/tools/csharp-ls"), "").unwrap();
        for program in ["marksman", "rust-analyzer", "gopls", "csharp-ls"] {
            assert!(linux.find(program).is_some(), "{program}");
        }
        assert_eq!(
            linux.find("marksman"),
            Some(home.join(".local/bin/marksman"))
        );
        // Go's own variables move its folder.
        linux.gobin = Some(dir.path().join("gobin"));
        assert!(linux.find("gopls").is_none());
        // The PATH comes first.
        let search: Vec<PathBuf> = std::env::split_paths(&linux.search_path()).collect();
        assert_eq!(search[0], dir.path().join("bin"));
        assert!(search.contains(&home.join(".local/bin")));
    }

    #[test]
    fn a_failure_says_why_in_words_to_act_on() {
        let said = |text: &str| explain("pyright", "npm install -g pyright", Some(1), text);
        assert_eq!(
            said("npm ERR! code EACCES\nnpm ERR! path /usr/lib/node_modules"),
            "Couldn't install pyright: it isn't allowed to write where it installs. Run it in a terminal"
        );
        assert!(said("sudo: a terminal is required to read the password").contains("sudo"));
        assert!(said("curl: (6) Could not resolve host: github.com").contains("no connection"));
        assert!(said("curl: (22) The requested URL returned error: 404").contains("isn't there"));
        assert_eq!(
            explain(
                "jdtls",
                "brew install jdtls",
                Some(127),
                "sh: brew: not found"
            ),
            "Couldn't install jdtls: brew isn't on the PATH"
        );
        assert_eq!(
            said("something\nthe last thing it said\n\n"),
            "Couldn't install pyright: the last thing it said"
        );
    }
}
