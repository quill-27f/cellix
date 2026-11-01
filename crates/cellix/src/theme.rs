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
    /// Matches of a search, other than the selected one.
    pub found: Rgb,
    pub row_hover: Rgb,
    pub row_active: Rgb,
    pub folder: Rgb,
    pub accent: Rgb,
    /// What wants the reviewer: an agent waiting, a failed step.
    pub attention: Rgb,
    pub keyword: Rgb,
    pub string: Rgb,
    pub comment: Rgb,
    pub function: Rgb,
    pub kind: Rgb,
    pub number: Rgb,
    pub added: Rgb,
    pub removed: Rgb,
    /// Stronger versions, for the words that changed within a line.
    pub added_word: Rgb,
    pub removed_word: Rgb,
    /// Lines that moved rather than changed.
    pub moved: Rgb,
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
    /// Background of a text box.
    pub input: Rgb,
    pub find_match: Rgb,
    /// Behind code blocks on a rendered Markdown page.
    pub code_bg: Rgb,
    /// The part of a minimap in view.
    pub minimap_view: Rgb,
    /// Hairlines between regions.
    pub border: Rgb,
    /// The border of a focused box.
    pub focus: Rgb,
    /// Floating panels such as the palette.
    pub popup: Rgb,
    pub popup_border: Rgb,
    /// The highlighted row of a list in a floating panel.
    pub pick: Rgb,
    pub button: Rgb,
    pub button_text: Rgb,
}

fn style(fg: Rgb, bg: Rgb) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

fn shifted(color: Rgb, by: (i16, i16, i16)) -> Rgb {
    let move_by = |channel: u8, by: i16| (i16::from(channel) + by).clamp(0, 255) as u8;
    Rgb(
        move_by(color.0, by.0),
        move_by(color.1, by.1),
        move_by(color.2, by.2),
    )
}

/// VS Code's Modern themes, since that's the look people already know. Surfaces are quiet
/// and the blue is kept for what's active.
impl Theme {
    /// Moves every surface by the same amount, so the editor sits exactly on the terminal's
    /// background and the panels keep their distance from it. Text colours stay, since
    /// they're chosen for contrast and a nearby background doesn't change that.
    pub fn on_background(mut self, background: Rgb) -> Self {
        let base = self.text.bg;
        let by = (
            i16::from(background.0) - i16::from(base.0),
            i16::from(background.1) - i16::from(base.1),
            i16::from(background.2) - i16::from(base.2),
        );
        // Ghostty's window padding shows the terminal's own colour, so chrome darker than it
        // would leave a seam around the app. Everything sits on one ground instead, and
        // the hairlines mark the regions.
        for surface in [
            &mut self.text,
            &mut self.sidebar,
            &mut self.bar,
            &mut self.tab,
            &mut self.tab_active,
            &mut self.gutter,
            &mut self.activity,
            &mut self.status,
            &mut self.breadcrumb,
        ] {
            surface.bg = background;
        }
        for color in [
            &mut self.row_hover,
            &mut self.row_active,
            &mut self.line,
            &mut self.guide,
            &mut self.scrollbar,
            &mut self.input,
            &mut self.code_bg,
            &mut self.minimap_view,
            &mut self.border,
            &mut self.popup,
            &mut self.popup_border,
        ] {
            *color = shifted(*color, by);
        }
        self
    }

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
        let panel = Rgb(248, 248, 249);
        Self {
            text: style(text, bg),
            sidebar: style(Rgb(59, 59, 62), panel),
            bar: style(Rgb(59, 59, 62), panel),
            tab: style(Rgb(110, 110, 115), panel),
            tab_active: style(Rgb(51, 51, 51), bg),
            gutter: style(Rgb(120, 120, 125), bg),
            selection: Rgb(173, 214, 255),
            found: Rgb(255, 236, 170),
            row_hover: Rgb(238, 238, 240),
            row_active: Rgb(214, 235, 255),
            folder: Rgb(120, 130, 150),
            accent: Rgb(0, 95, 184),
            attention: Rgb(170, 90, 0),
            keyword: Rgb(0, 0, 255),
            string: Rgb(163, 21, 21),
            comment: Rgb(0, 128, 0),
            function: Rgb(121, 94, 38),
            kind: Rgb(38, 127, 153),
            number: Rgb(9, 134, 88),
            added: Rgb(214, 245, 220),
            removed: Rgb(255, 224, 224),
            added_word: Rgb(165, 230, 180),
            removed_word: Rgb(250, 185, 185),
            moved: Rgb(226, 232, 250),
            activity: style(Rgb(120, 120, 126), panel),
            activity_active: Rgb(30, 30, 32),
            status: style(Rgb(59, 59, 62), panel),
            breadcrumb: style(Rgb(110, 110, 110), bg),
            line: Rgb(245, 245, 245),
            number_active: Rgb(40, 40, 45),
            guide: Rgb(211, 211, 211),
            scrollbar: Rgb(200, 200, 200),
            error: Rgb(225, 40, 40),
            heading: Rgb(29, 29, 31),
            git_modified: Rgb(137, 93, 0),
            git_added: Rgb(40, 130, 60),
            input: Rgb(255, 255, 255),
            find_match: Rgb(168, 216, 170),
            code_bg: Rgb(243, 243, 243),
            minimap_view: Rgb(236, 236, 240),
            border: Rgb(229, 229, 229),
            focus: Rgb(0, 144, 241),
            popup: Rgb(255, 255, 255),
            popup_border: Rgb(200, 200, 200),
            pick: Rgb(214, 235, 255),
            button: Rgb(0, 95, 184),
            button_text: Rgb(255, 255, 255),
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
        let bg = Rgb(31, 31, 32);
        let panel = Rgb(24, 24, 25);
        Self {
            text: style(Rgb(212, 212, 212), bg),
            sidebar: style(Rgb(204, 204, 204), panel),
            bar: style(Rgb(204, 204, 204), panel),
            tab: style(Rgb(150, 150, 150), panel),
            tab_active: style(Rgb(255, 255, 255), bg),
            gutter: style(Rgb(133, 133, 133), bg),
            selection: Rgb(38, 79, 120),
            found: Rgb(106, 84, 28),
            row_hover: Rgb(42, 45, 46),
            row_active: Rgb(55, 55, 61),
            folder: Rgb(144, 164, 174),
            accent: Rgb(55, 148, 255),
            attention: Rgb(240, 180, 90),
            keyword: Rgb(86, 156, 214),
            string: Rgb(206, 145, 120),
            comment: Rgb(106, 153, 85),
            function: Rgb(220, 220, 170),
            kind: Rgb(78, 201, 176),
            number: Rgb(181, 206, 168),
            added: Rgb(35, 61, 37),
            removed: Rgb(72, 32, 32),
            added_word: Rgb(46, 100, 54),
            removed_word: Rgb(120, 45, 45),
            moved: Rgb(38, 46, 70),
            activity: style(Rgb(134, 134, 138), panel),
            activity_active: Rgb(230, 230, 232),
            status: style(Rgb(204, 204, 204), panel),
            breadcrumb: style(Rgb(165, 165, 165), bg),
            line: Rgb(42, 45, 46),
            number_active: Rgb(198, 198, 198),
            guide: Rgb(64, 64, 64),
            scrollbar: Rgb(79, 79, 79),
            error: Rgb(244, 71, 71),
            heading: Rgb(240, 240, 240),
            git_modified: Rgb(226, 192, 141),
            git_added: Rgb(115, 201, 145),
            input: Rgb(60, 60, 60),
            find_match: Rgb(81, 92, 47),
            code_bg: Rgb(38, 38, 40),
            minimap_view: Rgb(50, 50, 54),
            border: Rgb(46, 46, 48),
            focus: Rgb(0, 127, 212),
            popup: Rgb(37, 37, 39),
            popup_border: Rgb(69, 69, 72),
            pick: Rgb(4, 57, 94),
            button: Rgb(0, 120, 212),
            button_text: Rgb(255, 255, 255),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_sits_on_the_terminal_background() {
        let blue = Rgb(30, 30, 46);
        let theme = Theme::default().on_background(blue);
        assert_eq!(theme.text.bg, blue);
        assert_eq!(theme.gutter.bg, blue);
        assert_eq!(
            theme.sidebar.bg, blue,
            "so no seam shows at the window's edge"
        );
        assert_eq!(theme.status.bg, blue);
    }

    fn brightness(color: Rgb) -> u32 {
        (color.0 as u32 + color.1 as u32 + color.2 as u32) / 3
    }

    /// A panel left in the other theme's colours stands out as a block.
    #[test]
    fn every_panel_belongs_to_its_theme() {
        let light = Theme::named(Some("light"));
        for panel in [
            light.text,
            light.sidebar,
            light.activity,
            light.tab,
            light.tab_active,
        ] {
            assert!(brightness(panel.bg) > 200, "{panel:?}");
        }
        let dark = Theme::default();
        for panel in [
            dark.text,
            dark.sidebar,
            dark.activity,
            dark.tab,
            dark.tab_active,
        ] {
            assert!(brightness(panel.bg) < 80, "{panel:?}");
        }
    }

    #[test]
    fn activity_icons_stay_readable() {
        for theme in [Theme::named(Some("light")), Theme::default()] {
            let bg = brightness(theme.activity.bg) as i32;
            assert!((brightness(theme.activity_active) as i32 - bg).abs() > 120);
            assert!((brightness(theme.activity.fg) as i32 - bg).abs() > 60);
        }
    }
}
