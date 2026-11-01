use cellix_lang::Token;
use cellix_render::{Rgb, Style};

/// Colours for the whole interface.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub text: Style,
    pub sidebar: Style,
    pub bar: Style,
    pub tab: Style,
    pub tab_active: Style,
    pub gutter: Style,
    pub selection: Rgb,
    pub row_hover: Rgb,
    pub accent: Rgb,
    pub keyword: Rgb,
    pub string: Rgb,
    pub comment: Rgb,
    pub function: Rgb,
    pub kind: Rgb,
    pub number: Rgb,
    pub added: Rgb,
    pub removed: Rgb,
    pub activity: Style,
    pub activity_active: Rgb,
    pub status: Style,
    pub breadcrumb: Style,
    /// Background of the line holding the cursor.
    pub line: Rgb,
    pub number_active: Rgb,
    pub guide: Rgb,
    pub scrollbar: Rgb,
    pub error: Rgb,
    pub heading: Rgb,
    pub git_modified: Rgb,
    pub git_added: Rgb,
}

fn style(fg: Rgb, bg: Rgb) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

/// VS Code's Dark+ and Light+ colours, since that's the look people already know.
impl Theme {
    /// A built-in theme by name. Unknown names get the default.
    pub fn named(name: Option<&str>) -> Self {
        match name {
            Some("light") => Self::light(),
            _ => Self::default(),
        }
    }

    fn light() -> Self {
        let text = Rgb(0, 0, 0);
        let bg = Rgb(255, 255, 255);
        let panel = Rgb(243, 243, 243);
        Self {
            text: style(text, bg),
            sidebar: style(Rgb(97, 97, 97), panel),
            bar: style(Rgb(97, 97, 97), Rgb(240, 240, 240)),
            tab: style(Rgb(110, 110, 110), Rgb(236, 236, 236)),
            tab_active: style(Rgb(51, 51, 51), bg),
            gutter: style(Rgb(35, 120, 145), bg),
            selection: Rgb(173, 214, 255),
            row_hover: Rgb(228, 230, 241),
            accent: Rgb(0, 95, 184),
            keyword: Rgb(0, 0, 255),
            string: Rgb(163, 21, 21),
            comment: Rgb(0, 128, 0),
            function: Rgb(121, 94, 38),
            kind: Rgb(38, 127, 153),
            number: Rgb(9, 134, 88),
            added: Rgb(214, 245, 220),
            removed: Rgb(255, 224, 224),
            activity: style(Rgb(120, 120, 120), Rgb(44, 44, 44)),
            activity_active: Rgb(255, 255, 255),
            status: style(Rgb(255, 255, 255), Rgb(0, 122, 204)),
            breadcrumb: style(Rgb(110, 110, 110), bg),
            line: Rgb(245, 245, 245),
            number_active: Rgb(11, 33, 110),
            guide: Rgb(211, 211, 211),
            scrollbar: Rgb(200, 200, 200),
            error: Rgb(225, 40, 40),
            heading: Rgb(0, 0, 128),
            git_modified: Rgb(137, 93, 0),
            git_added: Rgb(40, 130, 60),
        }
    }

    pub fn token(&self, token: Token) -> Rgb {
        match token {
            Token::Keyword => self.keyword,
            Token::String => self.string,
            Token::Comment => self.comment,
            Token::Function => self.function,
            Token::Type => self.kind,
            Token::Number | Token::Constant => self.number,
            Token::Attribute | Token::Heading => self.accent,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        let bg = Rgb(30, 30, 30);
        let panel = Rgb(37, 37, 38);
        Self {
            text: style(Rgb(212, 212, 212), bg),
            sidebar: style(Rgb(204, 204, 204), panel),
            bar: style(Rgb(204, 204, 204), Rgb(37, 37, 38)),
            tab: style(Rgb(150, 150, 150), Rgb(45, 45, 45)),
            tab_active: style(Rgb(255, 255, 255), bg),
            gutter: style(Rgb(133, 133, 133), bg),
            selection: Rgb(38, 79, 120),
            row_hover: Rgb(55, 55, 61),
            accent: Rgb(55, 148, 255),
            keyword: Rgb(86, 156, 214),
            string: Rgb(206, 145, 120),
            comment: Rgb(106, 153, 85),
            function: Rgb(220, 220, 170),
            kind: Rgb(78, 201, 176),
            number: Rgb(181, 206, 168),
            added: Rgb(35, 61, 37),
            removed: Rgb(72, 32, 32),
            activity: style(Rgb(133, 133, 133), Rgb(51, 51, 51)),
            activity_active: Rgb(255, 255, 255),
            status: style(Rgb(255, 255, 255), Rgb(0, 122, 204)),
            breadcrumb: style(Rgb(165, 165, 165), bg),
            line: Rgb(42, 45, 46),
            number_active: Rgb(198, 198, 198),
            guide: Rgb(64, 64, 64),
            scrollbar: Rgb(79, 79, 79),
            error: Rgb(244, 71, 71),
            heading: Rgb(86, 156, 214),
            git_modified: Rgb(226, 192, 141),
            git_added: Rgb(115, 201, 145),
        }
    }
}
