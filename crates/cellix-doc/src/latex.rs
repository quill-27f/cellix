//! What a paper needs that isn't on the page: which file to build, what to build it with,
//! what went wrong and where, and Markdown turned into LaTeX for export.

use std::path::{Path, PathBuf};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A problem from a build, at a line counting from one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

/// The file a build starts from: the one a `% !TEX root` comment names, or this one.
pub fn root_of(path: &Path, text: &str) -> PathBuf {
    let folder = path.parent().unwrap_or(Path::new(""));
    let named = text.lines().take(5).find_map(|line| {
        let rest = line.trim().strip_prefix('%')?.trim();
        let rest = rest.strip_prefix("!TEX")?.trim().strip_prefix("root")?;
        Some(rest.trim_start_matches([' ', '=']).trim().to_string())
    });
    named.map_or_else(|| path.to_path_buf(), |name| folder.join(name))
}

/// The engines Cellix knows, best first, as commands that take the file last.
pub const ENGINES: [&[&str]; 3] = [
    &["tectonic", "-X", "compile", "--synctex", "--keep-logs"],
    &[
        "latexmk",
        "-pdf",
        "-interaction=nonstopmode",
        "-file-line-error",
    ],
    &["pdflatex", "-interaction=nonstopmode", "-file-line-error"],
];

/// Finds `file:line: message` in a build's output. Every engine's errors reduce to this.
pub fn problems(output: &str, folder: &Path) -> Vec<Problem> {
    let mut found = Vec::new();
    for line in output.lines() {
        let line = line.strip_prefix("error: ").unwrap_or(line);
        let mut parts = line.splitn(3, ':');
        let (Some(file), Some(number), Some(message)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let file = file.trim();
        if !file.ends_with(".tex") && !file.ends_with(".bib") {
            continue;
        }
        let Ok(number) = number.trim().parse() else {
            continue;
        };
        found.push(Problem {
            file: folder.join(file.trim_start_matches("./")),
            line: number,
            message: message.trim().to_string(),
        });
    }
    found.dedup();
    found
}

/// `\bibitem`-style keys from a `.bib` file, for citing.
pub fn bib_keys(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix('@')?;
            let (kind, key) = rest.split_once('{')?;
            let skip = ["comment", "string", "preamble"];
            if skip.contains(&kind.trim().to_lowercase().as_str()) {
                return None;
            }
            let key = key.trim().trim_end_matches(',').trim();
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            other => out.push(other),
        }
    }
    out
}

/// A new table, ready to fill in.
pub fn new_table(rows: usize, columns: usize) -> String {
    let columns = columns.max(1);
    let row = |text: &dyn Fn(usize) -> String| {
        let cells: Vec<String> = (0..columns).map(text).collect();
        format!("    {} \\\\\n", cells.join(" & "))
    };
    let mut out = format!(
        "\\begin{{table}}[h]\n  \\centering\n  \\begin{{tabular}}{{{}}}\n    \\hline\n",
        "l".repeat(columns)
    );
    out.push_str(&row(&|c| format!("Column {}", c + 1)));
    out.push_str("    \\hline\n");
    for _ in 1..rows.max(2) {
        out.push_str(&row(&|_| String::new()));
    }
    out.push_str("    \\hline\n  \\end{tabular}\n  \\caption{}\n\\end{table}");
    out
}

const HEADER: &str = "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb,graphicx,hyperref}\n\\usepackage[margin=1in]{geometry}\n\\usepackage{booktabs}\n\\usepackage[normalem]{ulem}\n\\begin{document}\n";

/// Markdown as a LaTeX document. Math goes through untouched, which is the point.
pub fn from_markdown(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH;
    let mut out = String::from(HEADER);
    let mut verbatim = false;
    let mut columns = 0;
    let mut cell = 0;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => out.push_str(match level {
                HeadingLevel::H1 => "\n\\section*{",
                HeadingLevel::H2 => "\n\\subsection*{",
                HeadingLevel::H3 => "\n\\subsubsection*{",
                _ => "\n\\paragraph*{",
            }),
            Event::End(TagEnd::Heading(_)) => out.push_str("}\n"),
            Event::Start(Tag::Paragraph) => out.push('\n'),
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::Strong) => out.push_str("\\textbf{"),
            Event::Start(Tag::Emphasis) => out.push_str("\\emph{"),
            Event::Start(Tag::Strikethrough) => out.push_str("\\sout{"),
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
                out.push('}');
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                out.push_str(&format!("\\href{{{}}}{{", escape(&dest_url)));
            }
            Event::End(TagEnd::Link) => out.push('}'),
            Event::Start(Tag::Image { dest_url, .. }) => {
                out.push_str(&format!(
                    "\n\\includegraphics[width=\\linewidth]{{{dest_url}}}\n\\iffalse "
                ));
            }
            Event::End(TagEnd::Image) => out.push_str("\\fi\n"),
            Event::Start(Tag::BlockQuote(_)) => out.push_str("\n\\begin{quote}"),
            Event::End(TagEnd::BlockQuote(_)) => out.push_str("\\end{quote}\n"),
            Event::Start(Tag::List(None)) => out.push_str("\n\\begin{itemize}\n"),
            Event::Start(Tag::List(Some(_))) => out.push_str("\n\\begin{enumerate}\n"),
            Event::End(TagEnd::List(false)) => out.push_str("\\end{itemize}\n"),
            Event::End(TagEnd::List(true)) => out.push_str("\\end{enumerate}\n"),
            Event::Start(Tag::Item) => out.push_str("\\item "),
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::TaskListMarker(done) => {
                out.push_str(if done {
                    "[$\\boxtimes$] "
                } else {
                    "[$\\square$] "
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                verbatim = true;
                out.push_str("\n\\begin{verbatim}\n");
            }
            Event::End(TagEnd::CodeBlock) => {
                verbatim = false;
                out.push_str("\\end{verbatim}\n");
            }
            Event::Start(Tag::Table(aligns)) => {
                columns = aligns.len();
                let spec: String = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::Center => 'c',
                        Alignment::Right => 'r',
                        _ => 'l',
                    })
                    .collect();
                out.push_str(&format!("\n\\begin{{tabular}}{{{spec}}}\n\\toprule\n"));
            }
            Event::End(TagEnd::Table) => out.push_str("\\bottomrule\n\\end{tabular}\n"),
            Event::Start(Tag::TableHead | Tag::TableRow) => cell = 0,
            Event::End(TagEnd::TableHead) => out.push_str(" \\\\\n\\midrule\n"),
            Event::End(TagEnd::TableRow) => out.push_str(" \\\\\n"),
            Event::Start(Tag::TableCell) => {
                if cell > 0 && cell < columns {
                    out.push_str(" & ");
                }
                cell += 1;
            }
            Event::Text(text) if verbatim => out.push_str(&text),
            Event::Text(text) => out.push_str(&escape(&text)),
            Event::Code(text) => out.push_str(&format!("\\texttt{{{}}}", escape(&text))),
            Event::InlineMath(text) => out.push_str(&format!("${text}$")),
            Event::DisplayMath(text) => out.push_str(&format!("\n\\[\n{}\n\\]\n", text.trim())),
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("\\\\\n"),
            Event::Rule => out.push_str("\n\\noindent\\rule{\\linewidth}{0.4pt}\n"),
            _ => {}
        }
    }
    out.push_str("\n\\end{document}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_table_has_its_size() {
        let table = new_table(3, 2);
        assert!(table.contains("{ll}"));
        assert!(table.contains("Column 1 & Column 2"));
        assert_eq!(table.matches("\\\\\n").count(), 3, "{table}");
    }

    #[test]
    fn errors_are_found_with_their_lines() {
        let output = "note: Running TeX ...\nerror: main.tex:3: Undefined control sequence\nerror: halted on potentially-recoverable error as specified\n./sections/intro.tex:12: Missing $ inserted.\n";
        let found = problems(output, Path::new("/w"));
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].file, Path::new("/w/main.tex"));
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].message, "Undefined control sequence");
        assert_eq!(found[1].file, Path::new("/w/sections/intro.tex"));
    }

    #[test]
    fn a_root_comment_names_the_main_file() {
        let chapter = Path::new("/w/ch/one.tex");
        assert_eq!(root_of(chapter, "\\section{A}\n"), chapter);
        let text = "% !TEX root = ../main.tex\n\\section{A}\n";
        assert_eq!(root_of(chapter, text), Path::new("/w/ch/../main.tex"));
    }

    #[test]
    fn citation_keys_come_from_the_bibliography() {
        let bib = "@article{knuth84,\n title={x}\n}\n@comment{not this}\n@book{ lamport94 ,\n}\n";
        assert_eq!(bib_keys(bib), ["knuth84", "lamport94"]);
    }

    #[test]
    fn markdown_becomes_latex_with_math_untouched() {
        let tex = from_markdown(
            "# Title\n\nSome **bold** 50% text with $x_i^2$.\n\n$$\\frac{a}{b}$$\n\n| A | B |\n|---|--:|\n| 1 | 2 |\n\n- one\n\n```\ncode_here\n```\n",
        );
        assert!(tex.contains("\\section*{Title}"));
        assert!(tex.contains("\\textbf{bold}"));
        assert!(tex.contains("50\\% text"));
        assert!(tex.contains("$x_i^2$"), "math is left alone: {tex}");
        assert!(tex.contains("\\frac{a}{b}"));
        assert!(tex.contains("\\begin{tabular}{lr}"));
        assert!(tex.contains("1 & 2"));
        assert!(tex.contains("\\begin{itemize}"));
        assert!(tex.contains("code_here"), "verbatim isn't escaped");
    }
}
