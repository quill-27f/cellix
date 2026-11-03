//! YAML front matter at the top of a Markdown document. Exports set it as the title, author and
//! date, where it would otherwise print as a heading full of `key: value` lines.

/// What front matter says, as far as a page's title goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

impl Front {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.author.is_none() && self.date.is_none()
    }
}

/// The front matter and the document after it. A document without any is returned whole.
pub fn split(markdown: &str) -> (Front, &str) {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return (Front::default(), markdown);
    };
    let Some(end) = rest.find("\n---") else {
        return (Front::default(), markdown);
    };
    let after = &rest[end + 4..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let mut front = Front::default();
    for line in rest[..end].lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "title" => front.title = Some(value),
            "author" | "authors" => front.author = Some(value),
            "date" => front.date = Some(value),
            _ => {}
        }
    }
    (front, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_gives_a_title_and_the_rest() {
        let text = "---\ntitle: \"An RFC\"\nauthor: Ada\ntags: [a]\n---\n\n# Body\n";
        let (front, rest) = split(text);
        assert_eq!(front.title.as_deref(), Some("An RFC"));
        assert_eq!(front.author.as_deref(), Some("Ada"));
        assert_eq!(rest, "\n# Body\n");
    }

    #[test]
    fn a_document_without_it_is_left_alone() {
        let text = "# Title\n\n---\n\nmore\n";
        assert_eq!(split(text), (Front::default(), text));
    }
}
