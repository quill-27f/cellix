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
}

fn style(fg: Rgb, bg: Rgb) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
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
        }
    }
}
