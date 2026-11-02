//! LaTeX as an editable page. The file stays the document: its commands hide, what they mean
//! shows, and every visible character keeps its place in the source. Anything this doesn't
//! recognise shows as source, so nothing in a paper is ever out of reach.

use super::*;

const MATH: [&str; 11] = [
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "displaymath",
    "eqnarray",
    "eqnarray*",
];
const VERBATIM: [&str; 3] = ["verbatim", "lstlisting", "minted"];
const SECTIONS: [(&str, usize); 6] = [
    ("chapter", 1),
    ("section", 1),
    ("subsection", 2),
    ("subsubsection", 3),
    ("paragraph", 4),
    ("subparagraph", 5),
];
/// Commands that lay a page out, with nothing to show here.
const QUIET: [&str; 14] = [
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
];
const CITES: [&str; 10] = [
    "cite",
    "citep",
    "citet",
    "citeauthor",
    "citeyear",
    "autocite",
    "parencite",
    "textcite",
    "footcite",
    "nocite",
];
const REFS: [&str; 6] = ["ref", "eqref", "cref", "Cref", "autoref", "pageref"];
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
        // Not something with an argument: leave it as it's written.
        let plain = name.chars().count() + 1;
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
    let name = name.trim_end_matches('*');
    match name {
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
                    ..look.font
                },
                ..look
            };
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
            for (n, c) in inner.iter().enumerate() {
                out.push(glyph(*c, Some(start + n), link));
            }
            out.extend(decoration("]", Tone::Link));
        }
        _ if REFS.contains(&name) => {
            for (n, c) in inner.iter().enumerate() {
                out.push(glyph(*c, Some(start + n), link));
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
    equations: usize,
    figures: usize,
    tables: usize,
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

    fn heading(&mut self, index: usize, level: usize, range: (usize, usize)) {
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
        if numbered {
            self.equations += 1;
        }
        if open || context.cell().is_none() {
            let lines = &self.lines[first..=last];
            code_rows(lines, first, "", context, None, self.rows);
        }
        let Some(figure) = context.formula(inner, true) else {
            if !open && context.cell().is_some() {
                self.source(first, last);
            }
            return;
        };
        let at = self.rows.len();
        figure_rows(
            figure,
            context,
            (first, start + 1, end),
            self.width,
            self.rows,
        );
        if open {
            self.rows[at].stop = false;
        }
        if numbered {
            let mut number = decoration(&format!("({})", self.equations), Tone::Dim);
            measure_glyphs(&mut number, context.measure);
            let shown: f32 = number.iter().map(|g| g.advance).sum();
            let mut x = self.width - shown;
            for g in &mut number {
                g.x = x;
                x += g.advance;
            }
            self.rows[at].glyphs.extend(number);
        }
    }

    /// A list of what's in a `figure`: the image and its caption.
    fn figure(&mut self, first: usize, last: usize) {
        self.figures += 1;
        let number = self.figures;
        let image = (first..=last).find_map(|i| {
            let line = self.lines[i].1;
            let (start, end) = argument(line, "includegraphics")?;
            let chars: Vec<char> = line.chars().collect();
            Some(chars[start..end].iter().collect::<String>())
        });
        if let Some(path) = image {
            let shown = self.context.image(&path, self.width.min(90.0));
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
        for index in first..=last {
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
            wrap(template, glyphs, 0.0, self.width, self.rows);
            return;
        }
    }

    /// `tabular` as a real table: `&` and `\\` hide, and each cell stays editable.
    fn tabular(&mut self, first: usize, last: usize) {
        let mut parsed: Vec<Vec<Vec<Glyph>>> = Vec::new();
        let mut starts: Vec<Vec<usize>> = Vec::new();
        let mut places: Vec<(usize, usize)> = Vec::new();
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
        place_table(parsed, starts, &places, self.rows);
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
                && let Some(range) = argument(line, name.trim_end_matches('*'))
                    .or_else(|| argument(line, &format!("{name}*")))
            {
                self.heading(index, level, range);
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
        if text.starts_with("\\bibliography{") || text.starts_with("\\printbibliography") {
            self.label("References", index);
            return index + 1;
        }
        if let Some(name) = env_end(line) {
            if name == "itemize" || name == "enumerate" {
                self.lists.pop();
            }
            return index + 1;
        }
        if text.starts_with("\\item") {
            return self.item(index);
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
        match name {
            _ if MATH.contains(&name) => self.math(index, last, !name.ends_with('*')),
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
                self.tables += 1;
                let inner = (index..last).find(|n| env_begin(self.lines[*n].1) == Some("tabular"));
                let close = inner.and_then(|from| {
                    (from..=last).find(|n| self.lines[*n].1.contains("\\end{tabular}"))
                });
                if let (Some(from), Some(to)) = (inner, close) {
                    self.tabular(from, to);
                }
                self.caption("Table", self.tables, index, last);
            }
            "tabular" => self.tabular(index, last),
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
        let mut hidden = lead + "\\item".len();
        let chars: Vec<char> = line.chars().collect();
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
    let mut page = Page {
        lines,
        width,
        context,
        rows,
        equations: 0,
        figures: 0,
        tables: 0,
        lists: Vec::new(),
        title,
        author,
    };
    let mut index = match body {
        Some(found) => {
            if found > 0 {
                page.label(&format!("Preamble, {found} lines"), 0);
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
