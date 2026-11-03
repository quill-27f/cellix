//! LaTeX as an editable page. The file stays the document: its commands hide, what they mean
//! shows, and every visible character keeps its place in the source. Anything this doesn't
//! recognise shows as source, so nothing in a paper is ever out of reach.

use super::*;
use cellix_doc::latex::{CITES, Counters, REFS, equation_rows, is_math};

/// Space before a label beside a heading, in cells.
const LABEL_GAP: f32 = 0.8;

const VERBATIM: [&str; 3] = ["verbatim", "lstlisting", "minted"];
/// Sectioning commands, with their level for numbering. Paragraphs have no number.
const SECTIONS: [(&str, usize); 6] = [
    ("chapter", 0),
    ("section", 1),
    ("subsection", 2),
    ("subsubsection", 3),
    ("paragraph", 4),
    ("subparagraph", 5),
];
/// Commands that lay a page out, with nothing to show here.
const QUIET: [&str; 16] = [
    "\\centering",
    "\\noindent",
    "\\newpage",
    "\\clearpage",
    "\\bigskip",
    "\\medskip",
    "\\smallskip",
    "\\vfill",
    "\\hfill",
    "\\hline",
    "\\toprule",
    "\\midrule",
    "\\bottomrule",
    "\\pagebreak",
    "\\cmidrule",
    "\\cline",
];
/// Commands whose argument is just text here.
const TEXT_COMMANDS: [&str; 6] = ["textrm", "textsf", "textsc", "textnormal", "mbox", "text"];

fn env_begin(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("\\begin{")?;
    rest.split_once('}').map(|(name, _)| name)
}

fn env_end(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("\\end{")?;
    rest.split_once('}').map(|(name, _)| name)
}

/// The char index of the brace closing the one at `open`.
fn matching(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Finds `\name{...}` in a line, as the char range of its argument.
fn argument(line: &str, name: &str) -> Option<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let wanted: Vec<char> = format!("\\{name}").chars().collect();
    let start = (0..chars.len()).find(|i| chars[*i..].starts_with(&wanted))?;
    let mut open = start + wanted.len();
    if chars.get(open) == Some(&'[') {
        open += chars[open..].iter().position(|c| *c == ']')? + 1;
    }
    if chars.get(open) != Some(&'{') {
        return None;
    }
    Some((open + 1, matching(&chars, open)?))
}

fn bold(look: Look) -> Look {
    Look {
        font: Font {
            bold: true,
            ..look.font
        },
        ..look
    }
}

fn italic(look: Look) -> Look {
    Look {
        font: Font {
            italic: true,
            ..look.font
        },
        ..look
    }
}

fn dim(look: Look) -> Look {
    Look {
        tone: Tone::Dim,
        ..look
    }
}

/// Text with its commands hidden and what they mean applied.
pub(super) fn inline(
    chars: &[char],
    offset: usize,
    look: Look,
    context: &Context,
    out: &mut Vec<Glyph>,
) {
    let mut i = 0;
    while i < chars.len() {
        let at = offset + i;
        match chars[i] {
            '%' => {
                for (n, c) in chars[i..].iter().enumerate() {
                    out.push(glyph(*c, Some(at + n), dim(look)));
                }
                return;
            }
            '~' => {
                out.push(glyph(' ', Some(at), look));
                i += 1;
            }
            '-' if chars[i..].starts_with(&['-', '-', '-']) => {
                out.push(glyph('—', Some(at), look));
                i += 3;
            }
            '-' if chars[i..].starts_with(&['-', '-']) => {
                out.push(glyph('–', Some(at), look));
                i += 2;
            }
            '`' if chars[i..].starts_with(&['`', '`']) => {
                out.push(glyph('“', Some(at), look));
                i += 2;
            }
            '\'' if chars[i..].starts_with(&['\'', '\'']) => {
                out.push(glyph('”', Some(at), look));
                i += 2;
            }
            // A brace not yet closed is still being typed, so it shows.
            '{' if matching(chars, i).is_none() => {
                out.push(glyph('{', Some(at), dim(look)));
                i += 1;
            }
            '{' | '}' => i += 1,
            '$' => match math_end(&chars[i..]) {
                Some(end) => {
                    formula_span(&chars[i..=i + end], 1, at, context, out);
                    i += end + 1;
                }
                None => {
                    out.push(glyph('$', Some(at), look));
                    i += 1;
                }
            },
            '\\' => i += command(&chars[i..], at, look, context, out),
            c => {
                out.push(glyph(c, Some(at), look));
                i += 1;
            }
        }
    }
}

/// Handles the command at the start of `rest`, and returns how many chars it took.
fn command(rest: &[char], at: usize, look: Look, context: &Context, out: &mut Vec<Glyph>) -> usize {
    match rest.get(1) {
        Some('\\') => return 2,
        Some('(') => {
            let close = (2..rest.len()).find(|i| rest[*i - 1] == '\\' && rest[*i] == ')');
            if let Some(close) = close {
                formula_span(&rest[..=close], 2, at, context, out);
                return close + 1;
            }
        }
        Some(c) if "%&_#${}".contains(*c) => {
            out.push(glyph(*c, Some(at + 1), look));
            return 2;
        }
        _ => {}
    }
    let name: String = rest[1..]
        .iter()
        .take_while(|c| c.is_ascii_alphabetic() || **c == '*')
        .collect();
    let mut open = name.chars().count() + 1;
    if rest.get(open) == Some(&'[')
        && let Some(close) = rest[open..].iter().position(|c| *c == ']')
    {
        open += close + 1;
    }
    let arg = (rest.get(open) == Some(&'{'))
        .then(|| matching(rest, open))
        .flatten();
    let Some(close) = arg else {
        let symbol = match name.as_str() {
            "ldots" | "dots" => Some("…"),
            "S" => Some("§"),
            "LaTeX" => Some("LaTeX"),
            "TeX" => Some("TeX"),
            _ => None,
        };
        let plain = name.chars().count() + 1;
        if let Some(symbol) = symbol {
            for (n, c) in symbol.chars().enumerate() {
                out.push(glyph(c, (n == 0).then_some(at), look));
            }
            return plain;
        }
        // Not something with an argument: leave it as it's written.
        for (n, c) in rest[..plain.min(rest.len())].iter().enumerate() {
            out.push(glyph(*c, Some(at + n), dim(look)));
        }
        return plain;
    };
    let inner = &rest[open + 1..close];
    let start = at + open + 1;
    let link = Look {
        tone: Tone::Link,
        ..look
    };
    let missing = Look {
        tone: Tone::Failed,
        ..look
    };
    let name = name.trim_end_matches('*');
    let project = context.extras.project.as_deref();
    // The last of several arguments, which is the text in `\\multicolumn{2}{c}{Text}`.
    let last_argument = |mut from: usize, count: usize| {
        for _ in 1..count {
            if rest.get(from + 1) != Some(&'{') {
                return None;
            }
            from = matching(rest, from + 1)?;
        }
        Some(from)
    };
    match name {
        "thanks" => {}
        "multicolumn" | "multirow" => {
            let Some(end) = last_argument(close, 3) else {
                return close + 1;
            };
            let start = rest[..end].iter().rposition(|c| *c == '{').unwrap_or(end);
            inline(&rest[start + 1..end], at + start + 1, look, context, out);
            return end + 1;
        }
        "added" => {
            let tone = Look {
                tone: Tone::Inserted,
                ..look
            };
            inline(inner, start, tone, context, out);
        }
        "deleted" => {
            let tone = Look {
                tone: Tone::Deleted,
                ..look
            };
            inline(inner, start, tone, context, out);
        }
        "replaced" => {
            let second = (rest.get(close + 1) == Some(&'{'))
                .then(|| matching(rest, close + 1))
                .flatten();
            let Some(end) = second else {
                return close + 1;
            };
            let tone = |tone| Look { tone, ..look };
            let old = &rest[close + 2..end];
            inline(old, at + close + 2, tone(Tone::Deleted), context, out);
            inline(inner, start, tone(Tone::Inserted), context, out);
            return end + 1;
        }
        "textbf" => inline(inner, start, bold(look), context, out),
        "emph" | "textit" | "textsl" => inline(inner, start, italic(look), context, out),
        "underline" => {
            let lined = Look {
                underline: true,
                ..look
            };
            inline(inner, start, lined, context, out);
        }
        "texttt" => {
            let code = Look {
                tone: Tone::Code,
                font: Font {
                    mono: true,
                    ..look.font
                },
                ..look
            };
            for (n, c) in inner.iter().enumerate() {
                out.push(glyph(*c, Some(start + n), code));
            }
        }
        "url" => {
            for (n, c) in inner.iter().enumerate() {
                out.push(glyph(*c, Some(start + n), link));
            }
        }
        "href" => {
            let text = (rest.get(close + 1) == Some(&'{'))
                .then(|| matching(rest, close + 1))
                .flatten();
            let Some(end) = text else {
                return close + 1;
            };
            inline(&rest[close + 2..end], at + close + 2, link, context, out);
            return end + 1;
        }
        "label" => {
            let quiet = Look {
                tone: Tone::Dim,
                font: Font {
                    size: 80,
                    bold: false,
                    ..look.font
                },
                underline: false,
            };
            // Clear of what it labels, by more than a space that small.
            let mut gap = glyph(' ', None, quiet);
            gap.advance = LABEL_GAP;
            out.push(gap);
            for (n, c) in inner.iter().enumerate() {
                out.push(glyph(*c, Some(start + n), quiet));
            }
        }
        "footnote" => {
            out.extend(decoration(" (", Tone::Dim));
            inline(inner, start, dim(look), context, out);
            out.extend(decoration(")", Tone::Dim));
        }
        _ if CITES.contains(&name) => {
            out.extend(decoration("[", Tone::Link));
            let keys: String = inner.iter().collect();
            let mut first = true;
            for (n, key) in keys.split(',').map(str::trim).enumerate() {
                if n > 0 {
                    out.extend("; ".chars().map(|c| glyph(c, None, link)));
                }
                // A key the bibliography doesn't have prints as `[?]`, so it shows as missing.
                let (shown, tone) = match project.map(|p| p.citations.get(key)) {
                    Some(Some(shown)) => (shown.clone(), link),
                    Some(None) => (key.to_string(), missing),
                    None => (key.to_string(), link),
                };
                for c in shown.chars() {
                    out.push(glyph(c, first.then_some(start), tone));
                    first = false;
                }
            }
            out.extend(decoration("]", Tone::Link));
        }
        _ if REFS.contains(&name) => {
            let key: String = inner.iter().collect();
            let number = project.and_then(|p| p.labels.get(&key).cloned());
            match number {
                // What it will print as. The number has no source, so it holds the place.
                Some(number) => {
                    let kind = [
                        ("eq:", "Equation"),
                        ("sec:", "Section"),
                        ("fig:", "Figure"),
                        ("tab:", "Table"),
                        ("thm:", "Theorem"),
                        ("lem:", "Lemma"),
                    ];
                    let kind = kind.iter().find(|(prefix, _)| key.starts_with(prefix));
                    let shown = match (name, kind) {
                        ("eqref", _) => format!("({number})"),
                        ("cref" | "Cref", Some((_, kind))) => format!("{kind} {number}"),
                        _ => number,
                    };
                    for (n, c) in shown.chars().enumerate() {
                        let spot = (n == 0).then_some(start);
                        out.push(glyph(c, spot, link));
                    }
                }
                // A label nothing defines prints as `??`, so it shows as missing.
                None => {
                    for (n, c) in inner.iter().enumerate() {
                        out.push(glyph(*c, Some(start + n), missing));
                    }
                }
            }
        }
        _ if TEXT_COMMANDS.contains(&name) => inline(inner, start, look, context, out),
        _ => {
            // Unknown: show it whole, so the page never hides what it can't explain.
            for (n, c) in rest[..=close].iter().enumerate() {
                out.push(glyph(*c, Some(at + n), dim(look)));
            }
        }
    }
    close + 1
}

struct Page<'a, 'b> {
    lines: &'a [(usize, &'a str)],
    width: f32,
    context: &'a Context<'b>,
    rows: &'a mut Vec<Row>,
    /// Numbering so far, carrying on from the files before this one.
    counters: Counters,
    project: Rc<Project>,
    /// What starts the next paragraph, such as "Theorem 2."
    lead: Vec<Glyph>,
    /// For each list open here, its next number, if it's numbered.
    lists: Vec<Option<u64>>,
    /// Where `\title` and `\author` are, as (line, start, end) char ranges in those lines.
    title: Option<(usize, usize, usize)>,
    author: Option<(usize, usize, usize)>,
}

/// Whether a line starts something other than running text.
fn breaks_paragraph(line: &str) -> bool {
    let text = line.trim_start();
    text.is_empty()
        || text.starts_with("\\begin{")
        || text.starts_with("\\end{")
        || text.starts_with("\\item")
        || text.starts_with("\\[")
        || text.starts_with("$$")
        || text.starts_with('%')
        || [
            "\\input{",
            "\\include{",
            "\\bibliography",
            "\\printbibliography",
            "\\maketitle",
            "\\tableofcontents",
            "\\appendix",
        ]
        .iter()
        .any(|command| text.starts_with(command))
        || QUIET.iter().any(|q| text.starts_with(q))
        || SECTIONS
            .iter()
            .any(|(name, _)| text.starts_with(&format!("\\{name}")))
}

impl Page<'_, '_> {
    fn line_end(&self, index: usize) -> usize {
        let (offset, line) = self.lines[index];
        offset + line.chars().count()
    }

    fn label(&mut self, text: &str, index: usize) {
        let mut glyphs = decoration(text, Tone::Dim);
        for g in &mut glyphs {
            g.font.size = 85;
        }
        measure_glyphs(&mut glyphs, self.context.measure);
        let mut x = 0.0;
        for g in &mut glyphs {
            g.x = x;
            x += g.advance;
        }
        self.rows.push(Row {
            glyphs,
            ..spacer(index, self.lines[index].0)
        });
    }

    /// What the paper cites, numbered in the order it cites them, as the list prints.
    fn references(&mut self, index: usize) {
        let offset = self.lines[index].0;
        let known: Vec<(String, String)> = self
            .project
            .cited
            .iter()
            .filter_map(|key| Some((key.clone(), self.project.references.get(key)?.clone())))
            .collect();
        let mut known = known;
        // unsrt keeps the order of citing; the plain styles sort by author.
        let in_order = self.project.style.contains("unsrt");
        if !in_order {
            known.sort_by_key(|(_, reference)| reference.to_lowercase());
        }
        let missing = self
            .project
            .cited
            .iter()
            .filter(|key| !self.project.references.contains_key(*key));
        let mut shown: Vec<(String, bool)> = known
            .iter()
            .enumerate()
            .map(|(n, (_, reference))| (format!("[{}] {reference}", n + 1), true))
            .collect();
        // A key with no entry prints as [?], so it gets no number here either.
        shown.extend(missing.map(|key| (format!("[?] {key}: not in the bibliography"), false)));
        for (line, found) in shown {
            let tone = if found { Tone::Text } else { Tone::Failed };
            let mut glyphs = decoration(&line, tone);
            measure_glyphs(&mut glyphs, self.context.measure);
            let template = Row::new(index, offset);
            wrap(template, glyphs, 0.0, self.width, self.rows);
        }
    }

    /// The source of some lines, for what has no page form.
    fn source(&mut self, first: usize, last: usize) {
        let lines = &self.lines[first..=last];
        code_rows(lines, first, "", self.context, Some(Tone::Dim), self.rows);
    }

    /// A paragraph, running on across its source lines.
    fn paragraph(
        &mut self,
        first: usize,
        hidden: usize,
        look: Look,
        mut glyphs: Vec<Glyph>,
        indent: f32,
    ) -> usize {
        let decorated = glyphs.len();
        glyphs.append(&mut self.lead);
        let (offset, line) = self.lines[first];
        let chars: Vec<char> = line.chars().collect();
        inline(
            &chars[hidden..],
            offset + hidden,
            look,
            self.context,
            &mut glyphs,
        );
        let mut last = first;
        while last + 1 < self.lines.len() && !breaks_paragraph(self.lines[last + 1].1) {
            let newline = self.line_end(last);
            glyphs.extend(decoration(" ", Tone::Text));
            if let Some(space) = glyphs.last_mut() {
                space.at = Some(newline);
            }
            last += 1;
            let (start, text) = self.lines[last];
            let next: Vec<char> = text.chars().collect();
            let skip = next.iter().take_while(|c| c.is_whitespace()).count();
            inline(&next[skip..], start + skip, look, self.context, &mut glyphs);
        }
        measure_glyphs(&mut glyphs, self.context.measure);
        let mut lead = indent;
        if decorated > 0 {
            let width: f32 = glyphs[..decorated].iter().map(|g| g.advance).sum();
            glyphs[decorated - 1].advance += (indent + 1.8 - width).max(0.0);
            // A space stretched past its width can't be drawn as part of a run of text.
            glyphs[decorated - 1].tone = Tone::TableLine;
            lead = indent + 1.8;
        }
        let template = Row {
            lead,
            ..Row::new(first, self.line_end(last))
        };
        let start = self.rows.len();
        wrap(template, glyphs, lead, self.width, self.rows);
        let lines = self.lines;
        for row in &mut self.rows[start..] {
            if let Some(at) = row.glyphs.iter().find_map(|g| g.at) {
                row.line = lines
                    .partition_point(|(start, _)| *start <= at)
                    .saturating_sub(1);
            }
        }
        last + 1
    }

    fn heading(
        &mut self,
        index: usize,
        level: usize,
        range: (usize, usize),
        number: Option<String>,
    ) {
        let (offset, line) = self.lines[index];
        let chars: Vec<char> = line.chars().collect();
        let look = Look {
            tone: Tone::Heading,
            font: Font {
                bold: true,
                size: HEADINGS[level - 1],
                ..Font::default()
            },
            underline: false,
        };
        let mut glyphs = Vec::new();
        if let Some(number) = number {
            let mut mark = decoration(&format!("{number}  "), Tone::Heading);
            for g in &mut mark {
                g.font = look.font;
            }
            glyphs.extend(mark);
        }
        inline(
            &chars[range.0..range.1],
            offset + range.0,
            look,
            self.context,
            &mut glyphs,
        );
        let after = &chars[range.1 + 1..];
        inline(
            after,
            offset + range.1 + 1,
            SMALL_DIM,
            self.context,
            &mut glyphs,
        );
        measure_glyphs(&mut glyphs, self.context.measure);
        let tall = !self.context.measure.in_cells() && level <= 2;
        let template = Row {
            heading: level as u8,
            tall,
            ..Row::new(index, offset + chars.len())
        };
        wrap(template, glyphs, 0.0, self.width, self.rows);
        if tall {
            self.rows.push(spacer(index, offset));
            // A tall heading sits low in its rows, so text under it needs a row of air.
            let next = self.lines.get(index + 1);
            if next.is_some_and(|(_, line)| !line.trim().is_empty()) {
                self.rows.push(spacer(index, offset));
            }
        }
    }

    fn math(&mut self, first: usize, last: usize, numbered: bool) {
        let start = self.lines[first].0;
        let end = self.line_end(last);
        let context = self.context;
        context.extras.spans.borrow_mut().push((start, end + 1));
        let open = context.extras.open == Some((start, end + 1));
        let source: Vec<&str> = self.lines[first..=last].iter().map(|(_, l)| *l).collect();
        let source = source.join("\n");
        let inner = source.trim();
        let inner = inner
            .strip_prefix("\\[")
            .and_then(|t| t.strip_suffix("\\]"))
            .unwrap_or(inner);
        let numbers = equation_rows(&source, numbered, &mut self.counters.equations);
        if open || context.cell().is_none() {
            let lines = &self.lines[first..=last];
            code_rows(lines, first, "", context, None, self.rows);
        }
        let Some(figure) = context.formula_or_last(inner, true, start) else {
            if !open && context.cell().is_some() {
                self.source(first, last);
            }
            if let Some(reason) = context.formula_error(inner) {
                self.label(&format!("Can't typeset this: {reason}"), last);
            }
            return;
        };
        let at = self.rows.len();
        // A click lands in the formula, not on the line that opens its environment.
        let inside = if last > first {
            self.lines[first + 1].0
        } else {
            start + 1
        };
        figure_rows(figure, context, (first, inside, end), self.width, self.rows);
        if open {
            self.rows[at].stop = false;
        }
        let height = context.extras.figures.borrow()[figure].height;
        margin_numbers(
            (&numbers, &source),
            at,
            height,
            self.width,
            context,
            self.rows,
        );
    }

    /// Something only LaTeX can draw, as a picture once it's built and as source until then.
    fn drawn(&mut self, first: usize, last: usize) {
        let start = self.lines[first].0;
        let end = self.line_end(last);
        let context = self.context;
        context.extras.spans.borrow_mut().push((start, end + 1));
        let open = context.extras.open == Some((start, end + 1));
        let source: Vec<&str> = self.lines[first..=last].iter().map(|(_, l)| *l).collect();
        let built = context.built(&source.join("\n"), self.width);
        let shown = matches!(built, Built::Shown(_));
        if open || !shown {
            let lines = &self.lines[first..=last];
            code_rows(lines, first, "", context, Some(Tone::Dim), self.rows);
        }
        match built {
            Built::Shown(figure) => {
                let at = self.rows.len();
                let inside = self.lines[(first + 1).min(last)].0;
                figure_rows(figure, context, (first, inside, end), self.width, self.rows);
                if open {
                    self.rows[at].stop = false;
                }
            }
            Built::Waiting => self.label("Drawing with LaTeX…", last),
            Built::Failed(why) => self.label(&format!("Can't draw this: {why}"), last),
            Built::Never => {}
        }
    }

    /// An algorithm: a float, which can't be built alone, around steps that can. The page
    /// sets its caption, as it does a figure's.
    fn algorithm(&mut self, first: usize, last: usize) {
        self.counters.algorithms += 1;
        let number = self.counters.algorithms;
        let holds = |text: &str, wanted: &str| text.contains(wanted);
        let from = (first..=last).find(|n| holds(self.lines[*n].1, "\\begin{algorithmic}"));
        let to = (first..=last).find(|n| holds(self.lines[*n].1, "\\end{algorithmic}"));
        self.caption("Algorithm", number, first, last);
        match (from, to) {
            (Some(from), Some(to)) => self.drawn(from, to),
            _ => self.source(first, last),
        }
    }

    /// A list of what's in a `figure`: the image and its caption.
    fn figure(&mut self, first: usize, last: usize) {
        self.counters.figures += 1;
        let number = self.counters.figures;
        let image = (first..=last).find_map(|i| {
            let line = self.lines[i].1;
            let (start, end) = argument(line, "includegraphics")?;
            let chars: Vec<char> = line.chars().collect();
            let share = cellix_doc::figure::graphics_width(line);
            Some((chars[start..end].iter().collect::<String>(), share))
        });
        let drawn = (first..=last).find_map(|from| {
            let name = env_begin(self.lines[from].1)?;
            let close = format!("\\end{{{name}}}");
            let is_drawn = cellix_doc::latex::DRAWN.contains(&name);
            let to = (from..=last).find(|n| self.lines[*n].1.contains(&close))?;
            is_drawn.then_some((from, to))
        });
        if let (None, Some((from, to))) = (&image, drawn) {
            self.drawn(from, to);
        }
        if let Some((path, share)) = image {
            let shown = self.context.image(&path, fit_for(share, self.width));
            match shown {
                Some(figure) => {
                    let at = self.lines[first].0 + 1;
                    let end = self.line_end(last);
                    let place = (first, at, end);
                    figure_rows(figure, self.context, place, self.width, self.rows);
                }
                None => self.label(&format!("Image not found: {path}"), first),
            }
        }
        self.caption("Figure", number, first, last);
    }

    fn caption(&mut self, kind: &str, number: usize, first: usize, last: usize) {
        // The last one: a figure's own caption comes after those of its parts.
        for index in (first..=last).rev() {
            let line = self.lines[index].1;
            let Some((start, end)) = argument(line, "caption") else {
                continue;
            };
            let chars: Vec<char> = line.chars().collect();
            let mut glyphs = decoration(&format!("{kind} {number}: "), Tone::Dim);
            for g in &mut glyphs {
                g.font.bold = true;
                g.font.size = 90;
            }
            let look = Look {
                font: Font {
                    size: 90,
                    ..Font::default()
                },
                ..PLAIN
            };
            let offset = self.lines[index].0;
            inline(
                &chars[start..end],
                offset + start,
                look,
                self.context,
                &mut glyphs,
            );
            measure_glyphs(&mut glyphs, self.context.measure);
            let template = Row::new(index, offset + chars.len());
            let first = self.rows.len();
            wrap(template, glyphs, 0.0, self.width, self.rows);
            // Under a picture in the middle of the page, as a Markdown image's caption is. A
            // table sits at the page's left, so its caption stays with it there.
            if kind == "Table" {
                return;
            }
            for row in &mut self.rows[first..] {
                let reach = row.glyphs.last().map_or(0.0, |g| g.x + g.advance);
                let shift = ((self.width - reach) / 2.0).max(0.0);
                for g in &mut row.glyphs {
                    g.x += shift;
                }
                row.lead += shift;
            }
            return;
        }
    }

    /// `tabular` as a real table: `&` and `\\` hide, and each cell stays editable.
    fn tabular(&mut self, first: usize, last: usize) {
        let mut parsed: Vec<Vec<Vec<Glyph>>> = Vec::new();
        let mut starts: Vec<Vec<usize>> = Vec::new();
        let mut places: Vec<(usize, usize)> = Vec::new();
        let spec = argument(self.lines[first].1, "begin{tabular}");
        let aligns: Vec<char> = spec.map_or_else(Vec::new, |(start, end)| {
            let chars: Vec<char> = self.lines[first].1.chars().collect();
            let letters = chars[start..end].iter().copied();
            letters.filter(|c| "lcr".contains(*c)).collect()
        });
        for index in first + 1..last {
            let (offset, line) = self.lines[index];
            let trimmed = line.trim();
            if trimmed.is_empty() || QUIET.iter().any(|q| trimmed.starts_with(q)) {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let end = (0..chars.len())
                .find(|i| chars[*i] == '\\' && chars.get(*i + 1) == Some(&'\\'))
                .unwrap_or(chars.len());
            let header = parsed.is_empty();
            let look = if header { bold(PLAIN) } else { PLAIN };
            let (mut cells, mut cell_starts) = (Vec::new(), Vec::new());
            let mut start = 0;
            for i in 0..=end {
                let split = i == end || (chars[i] == '&' && (i == 0 || chars[i - 1] != '\\'));
                if !split {
                    continue;
                }
                let lead = chars[start..i]
                    .iter()
                    .take_while(|c| c.is_whitespace())
                    .count();
                let tail = chars[start..i]
                    .iter()
                    .rev()
                    .take_while(|c| c.is_whitespace())
                    .count();
                let from = start + lead;
                let to = i.saturating_sub(tail).max(from);
                let mut out = Vec::new();
                inline(
                    &chars[from..to],
                    offset + from,
                    look,
                    self.context,
                    &mut out,
                );
                measure_glyphs(&mut out, self.context.measure);
                cells.push(out);
                cell_starts.push(offset + from);
                start = i + 1;
            }
            parsed.push(cells);
            starts.push(cell_starts);
            places.push((index, offset + chars.len()));
        }
        place_table(parsed, starts, &places, &aligns, self.rows);
    }

    fn title_block(&mut self, index: usize) {
        if let Some((line, start, end)) = self.title {
            let saved = self.lines[line];
            let chars: Vec<char> = saved.1.chars().collect();
            let look = Look {
                tone: Tone::Heading,
                font: Font {
                    bold: true,
                    size: HEADINGS[0],
                    ..Font::default()
                },
                underline: false,
            };
            let mut glyphs = Vec::new();
            inline(
                &chars[start..end],
                saved.0 + start,
                look,
                self.context,
                &mut glyphs,
            );
            measure_glyphs(&mut glyphs, self.context.measure);
            let tall = !self.context.measure.in_cells();
            let template = Row {
                heading: 1,
                tall,
                ..Row::new(line, saved.0 + end)
            };
            wrap(template, glyphs, 0.0, self.width, self.rows);
            if tall {
                self.rows.push(spacer(index, saved.0));
            }
        }
        if let Some((line, start, end)) = self.author {
            let saved = self.lines[line];
            let chars: Vec<char> = saved.1.chars().collect();
            let mut glyphs = Vec::new();
            inline(
                &chars[start..end],
                saved.0 + start,
                dim(PLAIN),
                self.context,
                &mut glyphs,
            );
            measure_glyphs(&mut glyphs, self.context.measure);
            let template = Row::new(line, saved.0 + end);
            wrap(template, glyphs, 0.0, self.width, self.rows);
        }
        self.rows.push(spacer(index, self.lines[index].0));
    }

    fn lead_row(&mut self, mut glyphs: Vec<Glyph>, index: usize) {
        measure_glyphs(&mut glyphs, self.context.measure);
        let row = Row::new(index, self.lines[index].0);
        wrap(
            Row { stop: false, ..row },
            glyphs,
            0.0,
            self.width,
            self.rows,
        );
    }

    /// `\\input{file}`: a link to the file, since the page shows one file at a time.
    fn input(&mut self, index: usize, command: &str) {
        let (offset, line) = self.lines[index];
        let name = command.trim_start_matches('\\');
        let Some((start, end)) = argument(line, name) else {
            self.source(index, index);
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let file: String = chars[start..end].iter().collect();
        let mut glyphs = decoration(&format!("↳ {file}"), Tone::Link);
        measure_glyphs(&mut glyphs, self.context.measure);
        let mut x = 0.0;
        for g in &mut glyphs {
            g.x = x;
            x += g.advance;
        }
        let folder = self.context.extras.folder.unwrap_or(Path::new(""));
        let mut path = folder.join(&file);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        // What the file numbers, this one numbers on from.
        if let Some(after) = self.project.after.get(&path) {
            self.counters = after.clone();
        }
        self.rows.push(Row {
            glyphs,
            target: Some(Target::File(path)),
            ..spacer(index, offset)
        });
    }

    /// A theorem-like block: its number leads the first paragraph, and what's inside lays
    /// out as usual.
    fn theorem_start(&mut self, index: usize, name: &str, theorem: &cellix_doc::latex::Theorem) {
        let line = self.lines[index].1;
        let mut title = theorem.title.clone();
        if theorem.numbered {
            let counter = theorem.counter.clone();
            let count = self.counters.theorems.entry(counter).or_default();
            *count += 1;
            title = format!("{title} {count}");
        }
        let after = format!("\\begin{{{name}}}[");
        if let Some(at) = line.find(&after) {
            let rest = &line[at + after.len()..];
            if let Some((note, _)) = rest.split_once(']') {
                title = format!("{title} ({})", note.replace("--", "–"));
            }
        }
        let mut lead = decoration(&format!("{title}. "), Tone::Text);
        for g in &mut lead {
            g.font.bold = true;
        }
        self.lead = lead;
    }

    /// Lays out what starts at line `index`, and returns the line after it.
    fn block(&mut self, index: usize) -> usize {
        let (offset, line) = self.lines[index];
        let text = line.trim_start();
        if text.is_empty() {
            self.rows.push(Row::new(index, offset));
            return index + 1;
        }
        if text.starts_with('%') {
            self.source(index, index);
            return index + 1;
        }
        if QUIET.iter().any(|q| text.trim_end() == *q) {
            return index + 1;
        }
        for (name, level) in SECTIONS {
            let star = format!("\\{name}*{{");
            let plain = format!("\\{name}{{");
            if (text.starts_with(&star) || text.starts_with(&plain))
                && let Some(range) =
                    argument(line, name).or_else(|| argument(line, &format!("{name}*")))
            {
                let starred = text.starts_with(&star);
                let number = (!starred && level < 4).then(|| self.counters.heading(level));
                // With no chapters, sections are the top of the page.
                let chapters = self.counters.sections[0] > 0 || level == 0;
                let shown = (level + usize::from(chapters)).clamp(1, 6);
                self.heading(index, shown, range, number);
                return index + 1;
            }
        }
        if !self.lead.is_empty() && breaks_paragraph(line) {
            let glyphs = std::mem::take(&mut self.lead);
            self.lead_row(glyphs, index);
        }
        if text.starts_with("\\appendix") {
            self.counters.appendix = true;
            self.counters.sections = [0; 4];
            return index + 1;
        }
        for command in ["\\input", "\\include"] {
            if text.starts_with(&format!("{command}{{")) {
                self.input(index, command);
                return index + 1;
            }
        }
        if text.starts_with("\\maketitle") {
            self.title_block(index);
            return index + 1;
        }
        if text.starts_with("\\tableofcontents") {
            self.label("Table of contents", index);
            return index + 1;
        }
        // Says how references print, which the list below doesn't need.
        if text.starts_with("\\bibliographystyle{") {
            return index + 1;
        }
        if text.starts_with("\\bibliography{") || text.starts_with("\\printbibliography") {
            self.label("References", index);
            self.references(index);
            return index + 1;
        }
        if let Some(name) = env_end(line) {
            if name == "itemize" || name == "enumerate" || name == "thebibliography" {
                self.lists.pop();
            }
            if name == "proof" {
                let mut mark = decoration("∎", Tone::Dim);
                measure_glyphs(&mut mark, self.context.measure);
                if let Some(g) = mark.first_mut() {
                    g.x = (self.width - g.advance).max(0.0);
                }
                // It closes the proof's last line, unless that line runs out to the margin.
                let last = self.rows.iter_mut().rfind(|r| r.stop);
                let room = last.filter(|r| {
                    let reach = r.glyphs.last().map_or(0.0, |g| g.x + g.advance);
                    r.figure.is_none() && r.table.is_none() && reach + 2.0 < self.width
                });
                match room {
                    Some(row) => row.glyphs.extend(mark),
                    None => self.rows.push(Row {
                        glyphs: mark,
                        ..spacer(index, offset)
                    }),
                }
            }
            return index + 1;
        }
        if text.starts_with("\\item") || text.starts_with("\\bibitem") {
            return self.item(index);
        }
        if text.starts_with("$$") {
            let last = (index + 1..self.lines.len())
                .find(|n| self.lines[*n].1.contains("$$"))
                .filter(|_| text.trim_end() == "$$" || !text[2..].contains("$$"))
                .unwrap_or(index);
            self.math(index, last, false);
            return last + 1;
        }
        if text.starts_with("\\[") {
            let last = (index..self.lines.len())
                .find(|n| self.lines[*n].1.trim_end().ends_with("\\]"))
                .unwrap_or(index);
            self.math(index, last, false);
            return last + 1;
        }
        let Some(name) = env_begin(line) else {
            return self.paragraph(index, offset_of_text(line), PLAIN, Vec::new(), 0.0);
        };
        let close = format!("\\end{{{name}}}");
        let last = (index + 1..self.lines.len()).find(|n| self.lines[*n].1.contains(&close));
        let Some(last) = last else {
            self.source(index, index);
            return index + 1;
        };
        if let Some(theorem) = self.project.theorem(name) {
            self.theorem_start(index, name, &theorem);
            return index + 1;
        }
        match name {
            _ if is_math(name) => self.math(index, last, !name.ends_with('*')),
            "proof" => {
                let mut lead = decoration("Proof. ", Tone::Text);
                for g in &mut lead {
                    g.font.italic = true;
                }
                self.lead = lead;
                return index + 1;
            }
            "thebibliography" => {
                self.label("References", index);
                self.lists.push(Some(1));
                return index + 1;
            }
            "itemize" => {
                self.lists.push(None);
                return index + 1;
            }
            "enumerate" => {
                self.lists.push(Some(1));
                return index + 1;
            }
            "abstract" => {
                self.label("Abstract", index);
                return index + 1;
            }
            "quote" | "quotation" | "center" | "document" => return index + 1,
            "figure" | "figure*" => self.figure(index, last),
            "table" | "table*" => {
                self.counters.tables += 1;
                let inner = (index..last).find(|n| env_begin(self.lines[*n].1) == Some("tabular"));
                let close = inner.and_then(|from| {
                    (from..=last).find(|n| self.lines[*n].1.contains("\\end{tabular}"))
                });
                if let (Some(from), Some(to)) = (inner, close) {
                    self.tabular(from, to);
                }
                self.caption("Table", self.counters.tables, index, last);
            }
            "tabular" => self.tabular(index, last),
            "algorithm" | "algorithm*" => self.algorithm(index, last),
            _ if cellix_doc::latex::DRAWN.contains(&name) => self.drawn(index, last),
            _ if VERBATIM.contains(&name) => {
                let code = &self.lines[index + 1..last];
                code_rows(code, index + 1, "", self.context, None, self.rows);
            }
            _ => self.source(index, last),
        }
        last + 1
    }

    fn item(&mut self, index: usize) -> usize {
        let (_, line) = self.lines[index];
        let lead = line.chars().take_while(|c| c.is_whitespace()).count();
        let chars: Vec<char> = line.chars().collect();
        let mut hidden = lead + "\\item".len();
        // `\\bibitem{key}` has its key to step over too.
        if line.trim_start().starts_with("\\bibitem")
            && let Some((_, end)) = argument(line, "bibitem")
        {
            hidden = end + 1;
        }
        if chars.get(hidden) == Some(&' ') {
            hidden += 1;
        }
        let depth = self.lists.len().saturating_sub(1);
        let pad = "  ".repeat(depth);
        let marker = match self.lists.last_mut() {
            Some(Some(number)) => {
                let shown = format!("{pad}{number}. ");
                *number += 1;
                shown
            }
            _ => format!("{pad}{} ", ["•", "◦", "▪"][depth % 3]),
        };
        let tone = if marker.contains('.') {
            Tone::Dim
        } else {
            Tone::Text
        };
        let glyphs = decoration(&marker, tone);
        self.paragraph(index, hidden, PLAIN, glyphs, depth as f32 * 2.0)
    }
}

/// How many chars of indent a line has.
fn offset_of_text(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

const SMALL_DIM: Look = Look {
    tone: Tone::Dim,
    font: Font {
        mono: false,
        bold: false,
        italic: false,
        size: 80,
    },
    underline: false,
};

/// Lays out a whole paper.
pub(super) fn layout(lines: &[(usize, &str)], width: f32, context: &Context, rows: &mut Vec<Row>) {
    let body = lines
        .iter()
        .position(|(_, l)| l.contains("\\begin{document}"));
    let end = lines
        .iter()
        .position(|(_, l)| l.contains("\\end{document}"))
        .unwrap_or(lines.len());
    let place = |name: &str| {
        lines[..body.unwrap_or(0)]
            .iter()
            .enumerate()
            .find_map(|(i, (_, l))| {
                let (start, end) = argument(l, name)?;
                Some((i, start, end))
            })
    };
    let (title, author) = (place("title"), place("author"));
    let project = context.extras.project.clone().unwrap_or_default();
    let file = context.extras.file;
    let counters = file.and_then(|f| project.starts.get(f)).cloned();
    let mut page = Page {
        lines,
        width,
        context,
        rows,
        counters: counters.unwrap_or_default(),
        project,
        lead: Vec::new(),
        lists: Vec::new(),
        title,
        author,
    };
    let mut index = match body {
        Some(found) => {
            if context.extras.preamble {
                page.source(0, found.saturating_sub(1));
            } else if found > 0 {
                page.label(&format!("▸ Preamble, {found} lines"), 0);
                if let Some(row) = page.rows.last_mut() {
                    row.target = Some(Target::Preamble);
                }
            }
            found + 1
        }
        None => 0,
    };
    while index < end {
        index = page.block(index);
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use cellix_render::{CellMeasure, FontMeasure};

    fn text(row: &Row) -> String {
        row.glyphs.iter().map(|g| g.ch).collect()
    }

    fn page(source: &str) -> Vec<Row> {
        let extras = Extras {
            dialect: Dialect::Latex,
            project: Some(Rc::new(Project::from_text(source))),
            ..Extras::default()
        };
        layout_with(source, 70, &CellMeasure, &Registry::default(), &extras)
    }

    fn shown(source: &str) -> Vec<String> {
        page(source).iter().map(text).collect()
    }

    const PAPER: &str = r"\documentclass{article}
\usepackage{amsmath}
\title{On \emph{Attention}}
\author{Ada Lovelace}
\begin{document}
\maketitle
\begin{abstract}
We study attention.
\end{abstract}
\section{Introduction}\label{sec:intro}
Energy is \textbf{conserved} as in~\cite{knuth84}, see \ref{eq:sum}.
It runs on.

\begin{itemize}
  \item First point
  \item Second point
\end{itemize}
\begin{enumerate}
  \item One
  \item Two
\end{enumerate}
\begin{unknownthing}
keep me
\end{unknownthing}
\end{document}
ignored after the end
";

    #[test]
    fn latex_opens_as_an_editable_page() {
        let all = shown(PAPER).join("\n");
        assert!(all.contains("On Attention"), "the title: {all}");
        assert!(all.contains("Ada Lovelace"), "{all}");
        assert!(all.contains("Abstract"), "{all}");
        assert!(all.contains("Introduction"), "{all}");
        assert!(all.contains("conserved"), "{all}");
        assert!(!all.contains("\\textbf"), "{all}");
        assert!(all.contains("[knuth84]"), "{all}");
        assert!(all.contains("• First point"), "{all}");
        assert!(all.contains("2. Two"), "{all}");
        assert!(!all.contains("documentclass"), "{all}");
        assert!(!all.contains("ignored after"), "{all}");
    }

    #[test]
    fn what_isnt_understood_stays_visible() {
        let all = shown(PAPER).join("\n");
        assert!(all.contains("\\begin{unknownthing}"), "{all}");
        assert!(all.contains("keep me"), "{all}");
        let line = shown("A \\weird{thing} here\n").join("\n");
        assert!(line.contains("\\weird{thing}"), "{line}");
    }

    #[test]
    fn every_glyph_knows_its_source_position() {
        let source: Vec<char> = PAPER.chars().collect();
        for row in page(PAPER) {
            for g in &row.glyphs {
                if let Some(at) = g.at {
                    // A tie and a line's end both show as a space.
                    let shown = if "~\n".contains(source[at]) {
                        ' '
                    } else {
                        source[at]
                    };
                    assert_eq!(shown, g.ch, "at {at}");
                }
            }
        }
    }

    #[test]
    fn a_paragraph_runs_on_and_keeps_its_lines() {
        let rows = page("one two\nthree four\n\nnext\n");
        assert_eq!(text(&rows[0]), "one two three four");
        let space = rows[0].glyphs.iter().find(|g| g.at == Some(7)).unwrap();
        assert_eq!(space.ch, ' ', "the newline shows as a space");
    }

    #[test]
    fn hidden_marks_keep_escapes_and_spaces() {
        assert_eq!(shown("100\\% of a~b\n")[0], "100% of a b");
    }

    #[test]
    fn sections_and_references_show_their_numbers() {
        let source = "\\section{One}\n\\subsection{Inner}\\label{sec:inner}\n\\section{Two}\n\\begin{equation}\na\\label{eq:a}\n\\end{equation}\nSee \\ref{sec:inner} and \\eqref{eq:a} and \\ref{gone}.\n";
        let all = shown(source).join("\n");
        assert!(all.contains("1  One"), "{all}");
        assert!(all.contains("1.1  Inner"), "{all}");
        assert!(all.contains("2  Two"), "{all}");
        assert!(all.contains("See 1.1 and (1) and gone."), "{all}");
    }

    #[test]
    fn a_reference_to_nothing_shows_as_missing() {
        let source =
            "\\section{One}\\label{sec:one}\nSee \\ref{sec:one}, \\ref{gone} and \\cite{nobody}.\n";
        let failed: String = page(source)
            .iter()
            .flat_map(|row| &row.glyphs)
            .filter(|g| g.tone == Tone::Failed)
            .map(|g| g.ch)
            .collect();
        assert_eq!(failed, "gonenobody");
    }

    #[test]
    fn a_brace_being_typed_stays_on_the_page() {
        assert_eq!(shown("See \\cite{kn\n")[0], "See \\cite{kn");
    }

    #[test]
    fn a_tables_caption_stays_with_the_table_on_the_left() {
        let source = "\\begin{table}\n\\caption{Accuracy}\n\\begin{tabular}{ll}\na & b \\\\\n\\end{tabular}\n\\end{table}\n";
        let rows = page(source);
        let caption = rows
            .iter()
            .find(|r| text(r).contains("Accuracy"))
            .expect("a caption");
        assert!(caption.lead < 1.0, "left, with the table: {}", caption.lead);
    }

    #[test]
    fn sections_in_an_input_file_count_toward_the_paper() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("two.tex"), "\\section{Two}\n").unwrap();
        let root = dir.path().join("main.tex");
        let source =
            "\\begin{document}\n\\section{One}\n\\input{two}\n\\section{Three}\n\\end{document}\n";
        std::fs::write(&root, source).unwrap();
        let extras = Extras {
            dialect: Dialect::Latex,
            project: Some(Rc::new(Project::read(&root))),
            file: Some(&root),
            folder: Some(dir.path()),
            ..Extras::default()
        };
        let rows = layout_with(source, 70, &CellMeasure, &Registry::default(), &extras);
        let all = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        assert!(all.contains("3  Three"), "{all}");
    }

    #[test]
    fn a_papers_references_list_what_it_cites() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("refs.bib"),
            "@book{knuth84, author = {Knuth, Donald}, title = {The TeXbook}, year = 1984}\n",
        )
        .unwrap();
        let root = dir.path().join("main.tex");
        let source = "\\begin{document}\nSee \\cite{knuth84} and \\cite{nobody}.\n\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n";
        std::fs::write(&root, source).unwrap();
        let extras = Extras {
            dialect: Dialect::Latex,
            project: Some(Rc::new(Project::read(&root))),
            file: Some(&root),
            folder: Some(dir.path()),
            ..Extras::default()
        };
        let rows = layout_with(source, 70, &CellMeasure, &Registry::default(), &extras);
        let all = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        assert!(all.contains("References"), "{all}");
        assert!(
            all.contains("[1] Knuth, Donald (1984). The TeXbook."),
            "{all}"
        );
        assert!(all.contains("[?] nobody: not in the bibliography"), "{all}");
        assert!(!all.contains("bibliographystyle"), "{all}");
    }

    #[test]
    fn an_input_after_a_line_of_text_is_its_own() {
        let rows = shown("Some text here.\n\\input{sections/method}\n");
        let all = rows.join("\n");
        assert!(all.contains("Some text here."), "{all}");
        assert!(!all.contains("\\input"), "{all}");
        assert!(all.contains("↳ sections/method"), "{all}");
    }

    #[test]
    fn theorems_and_proofs_read_as_themselves() {
        let source = "\\newtheorem{thm}{Theorem}\n\\begin{document}\n\\begin{thm}[Big]\nIt holds.\n\\end{thm}\n\\begin{proof}\nObvious.\n\\end{proof}\n\\begin{lemma}\nAlso.\n\\end{lemma}\n\\end{document}\n";
        let all = shown(source).join("\n");
        assert!(all.contains("Theorem 1 (Big). It holds."), "{all}");
        assert!(all.contains("Proof. Obvious."), "{all}");
        assert!(all.contains('∎'), "{all}");
        assert!(all.contains("Lemma 1. Also."), "{all}");
        assert!(!all.contains("\\begin"), "{all}");
    }

    #[test]
    fn chapters_and_appendices_number_as_they_print() {
        let source = "\\chapter{One}\n\\section{A}\n\\appendix\n\\section{Extra}\n";
        let all = shown(source).join("\n");
        assert!(all.contains("1  One"), "{all}");
        assert!(all.contains("1.1  A"), "{all}");
        assert!(all.contains("A  Extra"), "{all}");
    }

    #[test]
    fn punctuation_reads_as_it_prints() {
        assert_eq!(shown("a---b--c ``q\'\' \\ldots\n")[0], "a—b–c “q” …");
    }

    #[test]
    fn a_table_keeps_its_alignment_and_merged_cells() {
        let source = "\\begin{tabular}{lr}\nName & Value \\\\\n\\cmidrule{1-2}\n\\multicolumn{2}{c}{Merged} \\\\\nA & 12345 \\\\\n\\end{tabular}\n";
        let rows = page(source);
        let table: Vec<&Row> = rows.iter().filter(|r| r.table.is_some()).collect();
        assert_eq!(table.len(), 3, "{:?}", shown(source));
        assert!(text(table[1]).contains("Merged"), "{}", text(table[1]));
        let x = |r: &Row, c: char| r.glyphs.iter().find(|g| g.ch == c).unwrap().x;
        assert!(x(table[0], 'V') > x(table[2], '1') - 0.1 || x(table[0], 'V') > 7.0);
    }

    #[test]
    fn an_input_is_a_link_to_its_file() {
        let rows = page("\\input{sections/one}\n");
        let row = rows.iter().find(|r| r.target.is_some()).unwrap();
        assert_eq!(
            row.target,
            Some(Target::File(PathBuf::from("sections/one.tex")))
        );
    }

    #[test]
    fn list_text_is_set_where_its_glyphs_say() {
        let rows = page("\\begin{itemize}\n\\item One \\textbf{two}\n\\end{itemize}\n");
        let row = rows.iter().find(|r| text(r).contains("One")).unwrap();
        let tones: Vec<Tone> = row.glyphs.iter().take(2).map(|g| g.tone).collect();
        assert_eq!(
            tones,
            [Tone::Text, Tone::TableLine],
            "the marker's gap isn't text"
        );
    }

    #[test]
    fn tables_show_as_tables() {
        let source = r"\begin{table}
\centering
\begin{tabular}{lr}
\toprule
Model & Accuracy \\
\midrule
Baseline & 91.2 \\
\bottomrule
\end{tabular}
\caption{Results here}
\end{table}
";
        let rows = page(source);
        let table: Vec<&Row> = rows.iter().filter(|r| r.table.is_some()).collect();
        assert_eq!(table.len(), 2, "{:?}", shown(source));
        assert!(table[0].table.as_ref().unwrap().header);
        let all = shown(source).join("\n");
        assert!(all.contains("Baseline"), "{all}");
        assert!(!all.contains('&'), "{all}");
        assert!(all.contains("Table 1: Results here"), "{all}");
    }

    #[test]
    fn equations_typeset_and_are_numbered() {
        let source = "\\begin{equation}\n  a = b \\label{eq:a}\n\\end{equation}\n\\begin{equation*}\n  c = d\n\\end{equation*}\n";
        let m = FontMeasure::new(17, 37);
        let extras = Extras {
            dialect: Dialect::Latex,
            project: Some(Rc::new(Project::from_text(source))),
            ..Extras::default()
        };
        let rows = layout_with(source, 70, &m, &Registry::default(), &extras);
        assert_eq!(extras.figures.into_inner().len(), 2);
        let spans = extras.spans.into_inner();
        assert_eq!(spans.len(), 2);
        let numbers: String = rows.iter().map(text).collect();
        assert!(numbers.contains("(1)"), "{numbers}");
        assert!(
            !numbers.contains("(2)"),
            "starred isn't numbered: {numbers}"
        );
    }
}
