//! A Jupyter notebook as a page. Its cells become Markdown that edits like any other page,
//! with a hidden line above each saying which cell it is, and saving puts the text back into
//! the JSON beside the outputs it already had. Cells don't run here: that's the kernel's work,
//! and a shell in the side pane can do it.

use base64::Engine;
use serde::Serialize;
use serde_json::{Value, json};

/// What a cell printed or drew.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Text(String),
    Error(String),
    Image(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notebook {
    json: Value,
    pub language: String,
    /// Cells waiting for or in the kernel.
    pub running: Vec<usize>,
}

/// The hidden line above a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Which cell of the file this is. None for one added since.
    pub cell: Option<usize>,
    pub code: bool,
}

/// Reads a hidden line.
pub fn marker(line: &str) -> Option<Marker> {
    let inner = line
        .trim()
        .strip_prefix("<!-- cell ")?
        .strip_suffix(" -->")?;
    let (cell, kind) = inner.split_once(' ')?;
    Some(Marker {
        cell: cell.parse().ok(),
        code: kind == "code",
    })
}

/// A new code cell's lines, in the language of the notebook.
pub fn new_code_cell(language: &str) -> String {
    format!("<!-- cell new code -->\n```{language}\n\n```")
}

/// A new text cell's lines.
pub fn new_text_cell() -> String {
    "<!-- cell new text -->\n".to_string()
}

/// A field that may be a string or a list of lines.
fn joined(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(lines)) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// Lines the way a notebook stores them, each keeping its newline.
fn stored(text: &str) -> Value {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    json!(lines)
}

fn no_colours(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for inner in chars.by_ref() {
                if inner.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn output_of(value: &Value) -> Option<Output> {
    match value.get("output_type")?.as_str()? {
        "stream" => Some(Output::Text(joined(value.get("text")))),
        "error" => {
            let trace = value.get("traceback").and_then(Value::as_array);
            let lines = trace.into_iter().flatten().filter_map(Value::as_str);
            Some(Output::Error(no_colours(
                &lines.collect::<Vec<_>>().join("\n"),
            )))
        }
        _ => {
            let data = value.get("data")?;
            let picture = ["image/png", "image/jpeg"]
                .iter()
                .find_map(|kind| data.get(*kind));
            if let Some(picture) = picture {
                let text: String = joined(Some(picture)).split_whitespace().collect();
                let bytes = base64::engine::general_purpose::STANDARD.decode(text);
                return bytes.ok().map(Output::Image);
            }
            data.get("text/plain")
                .map(|text| Output::Text(joined(Some(text))))
        }
    }
}

impl Notebook {
    pub fn read(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if json.get("cells").and_then(Value::as_array).is_none() {
            return Err("this has no cells".to_string());
        }
        let language = json
            .pointer("/metadata/kernelspec/language")
            .or_else(|| json.pointer("/metadata/language_info/name"))
            .and_then(Value::as_str)
            .unwrap_or("python")
            .to_string();
        Ok(Self {
            json,
            language,
            running: Vec::new(),
        })
    }

    /// The kernel's name, as the notebook gives it.
    pub fn kernel(&self) -> String {
        let name = self.json.pointer("/metadata/kernelspec/name");
        name.and_then(Value::as_str)
            .unwrap_or("python3")
            .to_string()
    }

    /// Clears what every cell printed.
    pub fn clear_all_outputs(&mut self) {
        let Some(cells) = self.json["cells"].as_array_mut() else {
            return;
        };
        for cell in cells.iter_mut().filter(|c| c.get("outputs").is_some()) {
            cell["outputs"] = json!([]);
            cell["execution_count"] = Value::Null;
        }
    }

    /// Clears what a cell last printed, as it starts again.
    pub fn clear_outputs(&mut self, cell: usize) {
        self.json["cells"][cell]["outputs"] = json!([]);
    }

    /// Adds something a running cell printed or drew, joining text to the text before it.
    pub fn add_output(&mut self, cell: usize, output: Value) {
        let Some(list) = self.json["cells"][cell]["outputs"].as_array_mut() else {
            return;
        };
        let stream = |value: &Value| value["output_type"] == "stream";
        if let Some(last) = list.last_mut()
            && stream(last)
            && stream(&output)
            && last["name"] == output["name"]
        {
            let text = joined(last.get("text")) + &joined(output.get("text"));
            last["text"] = json!(text);
            return;
        }
        list.push(output);
    }

    /// Notes the number a cell ran as.
    pub fn set_count(&mut self, cell: usize, count: Option<u64>) {
        self.json["cells"][cell]["execution_count"] = json!(count);
    }

    fn cells(&self) -> &[Value] {
        self.json["cells"].as_array().map_or(&[], Vec::as_slice)
    }

    /// The text the page edits.
    pub fn page(&self) -> String {
        let mut out = String::new();
        for (index, cell) in self.cells().iter().enumerate() {
            let source = joined(cell.get("source"));
            let source = source.trim_end();
            if cell.get("cell_type").and_then(Value::as_str) == Some("code") {
                let language = &self.language;
                out.push_str(&format!(
                    "<!-- cell {index} code -->\n```{language}\n{source}\n```\n\n"
                ));
            } else {
                out.push_str(&format!("<!-- cell {index} text -->\n{source}\n\n"));
            }
        }
        out
    }

    /// What a cell printed and drew when it last ran.
    pub fn outputs(&self, cell: usize) -> Vec<Output> {
        let cell = self.cells().get(cell);
        let outputs = cell
            .and_then(|c| c.get("outputs"))
            .and_then(Value::as_array);
        outputs
            .into_iter()
            .flatten()
            .filter_map(output_of)
            .collect()
    }

    /// The number a cell last ran as.
    pub fn count(&self, cell: usize) -> Option<u64> {
        self.cells().get(cell)?.get("execution_count")?.as_u64()
    }

    /// The notebook with the page's text in it, as Jupyter writes it.
    pub fn write(&self, page: &str) -> String {
        let mut cells = Vec::new();
        let mut current: Option<(Marker, Vec<&str>)> = None;
        let mut finish = |found: Option<(Marker, Vec<&str>)>| {
            if let Some((mark, lines)) = found {
                cells.push(self.cell_from(mark, &lines));
            }
        };
        for line in page.split('\n') {
            if let Some(mark) = marker(line) {
                finish(current.take());
                current = Some((mark, Vec::new()));
            } else if let Some((_, lines)) = &mut current {
                lines.push(line);
            }
        }
        finish(current.take());
        let mut out = self.json.clone();
        out["cells"] = Value::Array(cells);
        let mut bytes = Vec::new();
        let format = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut writer = serde_json::Serializer::with_formatter(&mut bytes, format);
        let written = out.serialize(&mut writer);
        match written {
            Ok(()) => format!("{}\n", String::from_utf8_lossy(&bytes)),
            Err(_) => self.json.to_string(),
        }
    }

    fn cell_from(&self, mark: Marker, lines: &[&str]) -> Value {
        let inside = if mark.code {
            let open = lines.iter().position(|l| l.starts_with("```"));
            let close = lines.iter().rposition(|l| l.trim() == "```");
            match (open, close) {
                (Some(open), Some(close)) if close > open => &lines[open + 1..close],
                _ => lines,
            }
        } else {
            lines
        };
        let text = inside.join("\n");
        let source = stored(text.trim_matches('\n'));
        let kept = mark.cell.and_then(|index| self.cells().get(index));
        match kept {
            Some(cell) => {
                let mut cell = cell.clone();
                cell["source"] = source;
                cell
            }
            None if mark.code => json!({
                "cell_type": "code",
                "execution_count": null,
                "metadata": {},
                "outputs": [],
                "source": source,
            }),
            None => json!({"cell_type": "markdown", "metadata": {}, "source": source}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SAMPLE: &str = r##"{
 "cells": [
  {"cell_type": "markdown", "metadata": {}, "source": ["# Training\n", "\n", "Words.\n"]},
  {"cell_type": "code", "execution_count": 3, "metadata": {"tags": ["x"]},
   "outputs": [
    {"output_type": "stream", "name": "stdout", "text": ["loss 0.5\n", "loss 0.2\n"]},
    {"output_type": "execute_result", "data": {"text/plain": ["42"]}, "metadata": {}, "execution_count": 3},
    {"output_type": "display_data", "data": {"image/png": "iVBORw0KGgo=\n", "text/plain": ["<Figure>"]}, "metadata": {}},
    {"output_type": "error", "ename": "E", "evalue": "v", "traceback": ["\u001b[31mBoom\u001b[0m", "line 2"]}
   ],
   "source": ["import torch\n", "print(1)"]}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4, "nbformat_minor": 5
}"##;

    #[test]
    fn a_notebook_reads_as_a_page() {
        let notebook = Notebook::read(SAMPLE).unwrap();
        let page = notebook.page();
        assert!(page.contains("# Training"));
        assert!(
            page.contains("```python\nimport torch\nprint(1)\n```"),
            "{page}"
        );
        assert_eq!(marker("<!-- cell 1 code -->").unwrap().cell, Some(1));
        assert!(marker("<!-- cell 1 code -->").unwrap().code);
        assert!(!marker("<!-- cell 0 text -->").unwrap().code);
    }

    #[test]
    fn outputs_come_out_in_order_without_terminal_colours() {
        let notebook = Notebook::read(SAMPLE).unwrap();
        let outputs = notebook.outputs(1);
        assert_eq!(outputs[0], Output::Text("loss 0.5\nloss 0.2\n".to_string()));
        assert_eq!(outputs[1], Output::Text("42".to_string()));
        assert!(matches!(outputs[2], Output::Image(_)));
        assert_eq!(outputs[3], Output::Error("Boom\nline 2".to_string()));
        assert_eq!(notebook.count(1), Some(3));
    }

    #[test]
    fn nothing_changed_is_nothing_lost() {
        let notebook = Notebook::read(SAMPLE).unwrap();
        let again: Value = serde_json::from_str(&notebook.write(&notebook.page())).unwrap();
        let original: Value = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(
            again["cells"][1]["outputs"],
            original["cells"][1]["outputs"]
        );
        assert_eq!(
            again["cells"][1]["metadata"],
            original["cells"][1]["metadata"]
        );
        assert_eq!(
            joined(Some(&again["cells"][0]["source"])),
            "# Training\n\nWords."
        );
        assert_eq!(again["nbformat_minor"], 5);
    }

    #[test]
    fn an_edit_reaches_its_cell_and_a_new_cell_is_added() {
        let notebook = Notebook::read(SAMPLE).unwrap();
        let page = notebook.page().replace("print(1)", "print(2)");
        let page = format!("{page}{}\n", new_code_cell("python"));
        let page = page.replace("```python\n\n```", "```python\nx = 1\n```");
        let written: Value = serde_json::from_str(&notebook.write(&page)).unwrap();
        let cells = written["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 3);
        assert_eq!(joined(Some(&cells[1]["source"])), "import torch\nprint(2)");
        assert_eq!(cells[1]["outputs"].as_array().unwrap().len(), 4);
        assert_eq!(joined(Some(&cells[2]["source"])), "x = 1");
        assert_eq!(cells[2]["cell_type"], "code");
    }

    #[test]
    fn a_cell_run_again_replaces_what_it_printed() {
        let mut notebook = Notebook::read(SAMPLE).unwrap();
        notebook.clear_outputs(1);
        let line = |text: &str| json!({"output_type": "stream", "name": "stdout", "text": text});
        notebook.add_output(1, line("one\n"));
        notebook.add_output(1, line("two\n"));
        notebook.set_count(1, Some(9));
        assert_eq!(
            notebook.outputs(1),
            [Output::Text("one\ntwo\n".to_string())]
        );
        assert_eq!(notebook.count(1), Some(9));
        let written: Value = serde_json::from_str(&notebook.write(&notebook.page())).unwrap();
        assert_eq!(written["cells"][1]["execution_count"], 9);
    }

    #[test]
    fn something_else_is_not_a_notebook() {
        assert!(Notebook::read("{}").is_err());
        assert!(Notebook::read("not json").is_err());
    }
}
