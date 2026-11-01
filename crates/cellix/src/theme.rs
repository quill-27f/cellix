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
}

fn style(fg: Rgb, bg: Rgb) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

impl Theme {
    /// A built-in theme by name. Unknown names get the default.
    pub fn named(name: Option<&str>) -> Self {
        match name {
            Some("light") => Self::light(),
            _ => Self::default(),
        }
    }

    fn light() -> Self {
        let text = Rgb(36, 41, 47);
        let bg = Rgb(255, 255, 255);
        let panel = Rgb(243, 244, 246);
        let dim = Rgb(100, 106, 115);
        Self {
            text: style(text, bg),
            sidebar: style(Rgb(60, 66, 74), panel),
            bar: style(Rgb(60, 66, 74), Rgb(228, 231, 235)),
            tab: style(dim, panel),
            tab_active: style(text, bg),
            gutter: style(Rgb(150, 156, 165), bg),
            selection: Rgb(200, 220, 250),
            row_hover: Rgb(226, 232, 240),
            accent: Rgb(9, 105, 218),
            keyword: Rgb(130, 40, 170),
            string: Rgb(10, 110, 60),
            comment: Rgb(106, 115, 125),
            function: Rgb(9, 90, 190),
            kind: Rgb(140, 90, 0),
            number: Rgb(190, 60, 20),
            added: Rgb(214, 245, 220),
            removed: Rgb(255, 224, 224),
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
        let text = Rgb(216, 219, 226);
        let dim = Rgb(120, 126, 140);
        let bg = Rgb(24, 26, 31);
        let panel = Rgb(19, 21, 25);
        Self {
            text: style(text, bg),
            sidebar: style(Rgb(176, 181, 192), panel),
            bar: style(Rgb(176, 181, 192), Rgb(32, 35, 42)),
            tab: style(dim, panel),
            tab_active: style(text, bg),
            gutter: style(Rgb(84, 90, 104), bg),
            selection: Rgb(52, 66, 96),
            row_hover: Rgb(38, 42, 52),
            accent: Rgb(110, 168, 254),
            keyword: Rgb(198, 146, 234),
            string: Rgb(165, 214, 155),
            comment: Rgb(106, 114, 130),
            function: Rgb(130, 170, 255),
            kind: Rgb(240, 198, 116),
            number: Rgb(247, 140, 108),
            added: Rgb(30, 58, 42),
            removed: Rgb(70, 34, 38),
        }
    }
}
