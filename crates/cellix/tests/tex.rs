//! The real binary building papers with a real TeX that isn't tectonic: latexmk, and plain
//! pdflatex with BibTeX passes. Run with `harness/x tex` where TeX Live is installed.
//!
//! Each test hides tectonic from the binary's PATH, and in the plain case latexmk too, by
//! giving it a folder of links to everything else on PATH.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const COLUMNS: u16 = 140;
const ROWS: u16 = 36;
/// A cold TeX on a CI machine can take a while over three passes and BibTeX.
const BUILD_TIME: Duration = Duration::from_secs(300);

struct Screen {
    parser: Arc<Mutex<vt100::Parser>>,
    input: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Screen {
    fn start(dir: &Path, path: &Path, home: &Path, file: &str) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_cellix"));
        command.cwd(dir);
        command.arg(file);
        command.env("PATH", path);
        // Nobody's own settings, such as a chosen engine, get in.
        command.env("HOME", home);
        command.env("XDG_CONFIG_HOME", home.join(".config"));
        command.env("CELLIX_TIER", "cells");
        let child = pty.slave.spawn_command(command).unwrap();
        let mut reader = pty.master.try_clone_reader().unwrap();
        let input = pty.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLUMNS, 0)));
        let sink = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().process(&buf[..n]);
            }
        });
        Self {
            parser,
            input,
            child,
        }
    }

    fn contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&self, text: &str, time: Duration) {
        let deadline = Instant::now() + time;
        while Instant::now() < deadline {
            let shown = self.contents();
            if shown.contains(text) {
                return;
            }
            // A failed build says so at once, so there's no waiting out the clock for it.
            for failed in ["The build failed", "needs a LaTeX engine"] {
                assert!(!shown.contains(failed), "{failed}:\n{shown}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("{text:?} never appeared:\n{}", self.contents());
    }

    fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).unwrap();
        self.input.flush().unwrap();
    }

    /// Runs a command from the palette, as a person would.
    fn command(&mut self, name: &str) {
        self.send(b"\x1bOP");
        std::thread::sleep(Duration::from_millis(400));
        self.send(name.as_bytes());
        std::thread::sleep(Duration::from_millis(400));
        self.send(b"\r");
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// A folder of links to every program on PATH but `hidden`, to stand in for PATH.
fn path_without(hidden: &[&str], folder: &Path) -> PathBuf {
    let bin = folder.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let link = bin.join(&name);
            if hidden.iter().any(|h| name == *h) || link.exists() || link.is_symlink() {
                continue;
            }
            let _ = std::os::unix::fs::symlink(entry.path(), link);
        }
    }
    for needed in ["pdflatex", "bibtex", "perl", "latexmk"] {
        if hidden.contains(&needed) {
            continue;
        }
        assert!(
            bin.join(needed).exists(),
            "{needed} must be installed to run these tests"
        );
    }
    for gone in hidden {
        assert!(!bin.join(gone).exists());
    }
    bin
}

/// A paper over two files, citing a `.bib`, with references each way between the files.
fn paper(dir: &Path) {
    std::fs::create_dir_all(dir.join("sections")).unwrap();
    std::fs::write(
        dir.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\n\\section{Introduction}\\label{sec:intro}\nAs \\cite{knuth84} shows, the method is in Section~\\ref{sec:method}.\n\\input{sections/method}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sections/method.tex"),
        "\\section{Method}\\label{sec:method}\nIt builds on Section~\\ref{sec:intro} and \\cite{lamport94}.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("refs.bib"),
        "@book{knuth84,\n  author = {Donald E. Knuth},\n  title = {The {\\TeX}book},\n  publisher = {Addison-Wesley},\n  year = {1984}\n}\n\n@book{lamport94,\n  author = {Leslie Lamport},\n  title = {{\\LaTeX}: A Document Preparation System},\n  publisher = {Addison-Wesley},\n  year = {1994}\n}\n",
    )
    .unwrap();
}

/// Builds the paper through the real binary and checks what TeX left, and that latexmk ran
/// or didn't, as `latexmk` says.
fn builds_a_paper(hidden: &[&str], latexmk: bool) {
    let scratch = tempfile::tempdir().unwrap();
    let bin = path_without(hidden, scratch.path());
    let home = scratch.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = scratch.path().join("paper");
    paper(&dir);
    let mut screen = Screen::start(&dir, &bin, &home, "main.tex");
    screen.wait_for("Introduction", Duration::from_secs(15));
    screen.command("build pdf");
    screen.wait_for("Built in", BUILD_TIME);
    assert!(dir.join("main.pdf").exists(), "no PDF");
    assert!(dir.join("main.synctex.gz").exists(), "no SyncTeX");
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    for unresolved in [
        "LaTeX Warning: Citation",
        "LaTeX Warning: Reference",
        "There were undefined references",
        "Rerun to get",
        "??",
    ] {
        assert!(!log.contains(unresolved), "{unresolved}:\n{log}");
    }
    let bbl = std::fs::read_to_string(dir.join("main.bbl")).expect("BibTeX ran");
    assert!(
        bbl.contains("knuth84") && bbl.contains("lamport94"),
        "{bbl}"
    );
    let aux = std::fs::read_to_string(dir.join("main.aux")).unwrap();
    assert!(aux.contains("\\bibcite{knuth84}"), "{aux}");
    assert_eq!(
        dir.join("main.fdb_latexmk").exists(),
        latexmk,
        "which engine ran"
    );
}

/// Exports Markdown to PDF through the real binary, and checks nothing of the build is left.
fn exports_markdown(hidden: &[&str]) {
    let scratch = tempfile::tempdir().unwrap();
    let bin = path_without(hidden, scratch.path());
    let home = scratch.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = scratch.path().join("notes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("note.md"),
        "# Attention\n\nThe weights sum to one, as $\\sum_i w_i = 1$ says.\n\n- One point\n- Another, with a [link](https://example.com)\n\n$$\nE = mc^2\n$$\n",
    )
    .unwrap();
    let mut screen = Screen::start(&dir, &bin, &home, "note.md");
    screen.wait_for("Attention", Duration::from_secs(15));
    screen.command("export to pdf");
    screen.wait_for("Built in", BUILD_TIME);
    drop(screen);
    assert!(
        dir.join("note.pdf").exists(),
        "the PDF lands beside the note"
    );
    let pdf = std::fs::read(dir.join("note.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF"), "a real PDF");
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "note.md" && name != "note.pdf" && name != ".cellix")
        .collect();
    assert!(left.is_empty(), "the build left {left:?}");
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn latexmk_builds_a_paper_without_tectonic() {
    builds_a_paper(&["tectonic"], true);
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn pdflatex_builds_a_paper_with_bibtex_passes() {
    builds_a_paper(&["tectonic", "latexmk"], false);
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn latexmk_exports_markdown_to_pdf() {
    exports_markdown(&["tectonic"]);
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn pdflatex_exports_markdown_to_pdf() {
    exports_markdown(&["tectonic", "latexmk"]);
}

/// Writes a paper's files, opens `open` in the real binary with `hidden` off PATH, and runs a
/// palette command, ready to wait on what the screen says.
fn start_with(
    hidden: &[&str],
    files: &[(&str, &str)],
    open: &str,
    command: &str,
) -> (tempfile::TempDir, PathBuf, Screen) {
    let scratch = tempfile::tempdir().unwrap();
    let bin = path_without(hidden, scratch.path());
    let home = scratch.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = scratch.path().join("paper");
    for (name, text) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut screen = Screen::start(&dir, &bin, &home, open);
    screen.wait_for(open, Duration::from_secs(15));
    screen.command(command);
    (scratch, dir, screen)
}

const REFS: &str = "@book{knuth84,\n  author = {Donald E. Knuth},\n  title = {The {\\TeX}book},\n  publisher = {Addison-Wesley},\n  year = {1984}\n}\n";

#[test]
#[ignore = "needs TeX Live and Biber: harness/x tex"]
fn pdflatex_runs_biber_for_a_biblatex_paper() {
    let main = "\\documentclass{article}\n\\usepackage[backend=biber]{biblatex}\n\\addbibresource{refs.bib}\n\\begin{document}\nAs \\cite{knuth84} shows.\n\\printbibliography\n\\end{document}\n";
    let files = [("main.tex", main), ("refs.bib", REFS)];
    let (_scratch, dir, screen) =
        start_with(&["tectonic", "latexmk"], &files, "main.tex", "build pdf");
    screen.wait_for("Built in", BUILD_TIME);
    assert!(
        !screen.contents().contains("failed"),
        "{}",
        screen.contents()
    );
    let bbl = std::fs::read_to_string(dir.join("main.bbl")).expect("Biber ran");
    assert!(bbl.contains("knuth84"), "{bbl}");
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    for unresolved in [
        "LaTeX Warning: Citation",
        "There were undefined",
        "Please rerun",
        "Rerun to get",
    ] {
        assert!(!log.contains(unresolved), "{unresolved}:\n{log}");
    }
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn pdflatex_settles_labels_in_included_chapters() {
    let main = "\\documentclass{article}\n\\begin{document}\n\\include{ch1}\n\\include{ch2}\n\\end{document}\n";
    let files = [
        ("main.tex", main),
        (
            "ch1.tex",
            "\\section{One}\\label{sec:one}\nSee Section~\\ref{sec:two}.\n",
        ),
        (
            "ch2.tex",
            "\\section{Two}\\label{sec:two}\nSee Section~\\ref{sec:one}.\n",
        ),
    ];
    let (_scratch, dir, screen) =
        start_with(&["tectonic", "latexmk"], &files, "main.tex", "build pdf");
    screen.wait_for("Built in", BUILD_TIME);
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    for unresolved in [
        "LaTeX Warning: Reference",
        "There were undefined",
        "Rerun to get",
    ] {
        assert!(!log.contains(unresolved), "{unresolved}:\n{log}");
    }
    assert!(
        screen.contents().contains("Built in") && !screen.contents().contains("warning"),
        "{}",
        screen.contents()
    );
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn pdflatex_says_when_bibtex_finds_no_bib() {
    let main = "\\documentclass{article}\n\\begin{document}\nAs \\cite{knuth84} shows.\n\\bibliographystyle{plain}\n\\bibliography{missing}\n\\end{document}\n";
    let (_scratch, _dir, screen) = start_with(
        &["tectonic", "latexmk"],
        &[("main.tex", main)],
        "main.tex",
        "build pdf",
    );
    screen.wait_for("BibTeX failed", BUILD_TIME);
    assert!(
        screen.contents().contains("missing.bib"),
        "{}",
        screen.contents()
    );
}

/// One undefined reference in a section file is one warning, however many passes latexmk
/// makes, and it belongs to that file, so F8 in the root finds nothing.
#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn latexmk_counts_a_warning_in_a_section_file_once() {
    let main = "\\documentclass{article}\n\\begin{document}\n\\section{Intro}\\label{sec:intro}\nText.\n\\input{sections/method}\n\\end{document}\n";
    let method = "\\section{Method}\nIt builds on Section~\\ref{sec:intro}.\n\nAnd on Section~\\ref{sec:gone}.\n";
    let files = [("main.tex", main), ("sections/method.tex", method)];
    let (_scratch, _dir, mut screen) = start_with(&["tectonic"], &files, "main.tex", "build pdf");
    screen.wait_for("Built in", BUILD_TIME);
    screen.wait_for("with a warning", Duration::from_secs(5));
    // F8, the next problem.
    screen.send(b"\x1b[19~");
    screen.wait_for("No problems in this file", Duration::from_secs(10));
}

#[test]
#[ignore = "needs TeX Live: harness/x tex"]
fn a_failed_pdflatex_export_keeps_the_last_good_pdf() {
    let files = [
        ("note.md", "# Note\n\nBroken: $\\nosuchmacro{x}$.\n"),
        ("note.pdf", "the good one"),
    ];
    let (_scratch, dir, screen) =
        start_with(&["tectonic", "latexmk"], &files, "note.md", "export to pdf");
    screen.wait_for("The PDF wasn't made", BUILD_TIME);
    assert_eq!(
        std::fs::read_to_string(dir.join("note.pdf")).unwrap(),
        "the good one"
    );
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "note.md" && name != "note.pdf" && name != ".cellix")
        .collect();
    assert!(left.is_empty(), "the build left {left:?}");
}
