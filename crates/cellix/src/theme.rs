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
    /// The wash on a row under the pointer, on the sidebar.
    pub row_hover: Rgb,
    /// The same on the sheet, and on a floating panel.
    pub page_hover: Rgb,
    pub popup_hover: Rgb,
    pub row_active: Rgb,
    pub accent: Rgb,
    /// A wash of the accent, behind what's selected.
    pub tint: Rgb,
    /// A wash of green, behind a hunk's Accept.
    pub accept_tint: Rgb,
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
    pub status: Style,
    /// The bar of workspaces under the status bar. It's the bottom row, so it's the
    /// terminal's own colour like the status bar, and its tiles stand out as chips.
    pub workspaces: Style,
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
    /// Secondary text on a floating panel, such as shortcuts.
    pub popup_dim: Rgb,
    /// What can't be done right now.
    pub disabled: Rgb,
    /// A gitignored file's name and icon in the tree: there, but quiet.
    pub ignored: Rgb,
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
    /// Text a tracked change adds.
    pub fn inserted(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x1a, 0x7f, 0x37)
        } else {
            Rgb(0x56, 0xd3, 0x64)
        }
    }

    /// Text a tracked change takes away.
    pub fn deleted(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xcf, 0x22, 0x2e)
        } else {
            Rgb(0xf8, 0x51, 0x49)
        }
    }

    /// What highlighted text stands on.
    pub fn highlighter(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xff, 0xec, 0x9e)
        } else {
            Rgb(0x6b, 0x5a, 0x1d)
        }
    }

    /// Bracket pairs by depth: gold, orchid and sky, as VS Code's, toned to sit with the
    /// syntax colours. The light ones pass 4.5:1 on white.
    pub fn brackets(&self) -> [Rgb; 3] {
        if self.is_light() {
            [
                Rgb(0x9a, 0x67, 0x00),
                Rgb(0xa2, 0x34, 0x9f),
                Rgb(0x1f, 0x63, 0xc7),
            ]
        } else {
            [
                Rgb(0xe6, 0xc0, 0x7b),
                Rgb(0xd2, 0x9f, 0xe3),
                Rgb(0x6c, 0xb6, 0xff),
            ]
        }
    }

    /// What the caret's matching brackets stand on, part way from the text to a selection.
    pub fn bracket_match(&self) -> Rgb {
        let mix = |a: u8, b: u8| ((u16::from(a) * 3 + u16::from(b) * 2) / 5) as u8;
        let (s, bg) = (self.selection, self.text.bg);
        Rgb(mix(s.0, bg.0), mix(s.1, bg.1), mix(s.2, bg.2))
    }

    /// A folded region's chip: lighter than both the text and the caret's line, so it shows
    /// on either.
    pub fn fold_chip(&self) -> Rgb {
        if self.is_light() {
            Rgb(0xdc, 0xe0, 0xe8)
        } else {
            Rgb(0x42, 0x42, 0x66)
        }
    }

    /// The ring around the caret's matching brackets, on the pixel tiers.
    pub fn bracket_ring(&self) -> Rgb {
        if self.is_light() {
            Rgb(0x9d, 0xb3, 0xe6)
        } else {
            Rgb(0x6b, 0x74, 0xa8)
        }
    }

    pub fn is_light(&self) -> bool {
        let Rgb(r, g, b) = self.text.bg;
        u16::from(r) + u16::from(g) + u16::from(b) > 3 * 128
    }

    /// Moves every surface by the same amount, so the frame sits exactly on the terminal's
    /// background and the panels keep their distance from it. Text colours stay, since
    /// they're chosen for contrast and a nearby background doesn't change that.
    pub fn on_background(mut self, background: Rgb) -> Self {
        let base = self.activity.bg;
        let by = (
            i16::from(background.0) - i16::from(base.0),
            i16::from(background.1) - i16::from(base.1),
            i16::from(background.2) - i16::from(base.2),
        );
        // The rail and the status bar are the terminal's own colour, so its window padding
        // can't show as a seam around them.
        for surface in [
            &mut self.sidebar,
            &mut self.bar,
            &mut self.tab,
            &mut self.activity,
            &mut self.status,
            &mut self.workspaces,
            &mut self.text,
            &mut self.tab_active,
            &mut self.gutter,
            &mut self.breadcrumb,
        ] {
            surface.bg = shifted(surface.bg, by);
        }
        for color in [
            &mut self.row_hover,
            &mut self.page_hover,
            &mut self.popup_hover,
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
            &mut self.tint,
            &mut self.accept_tint,
            &mut self.pick,
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
        let text = Rgb(38, 38, 46);
        let bg = Rgb(255, 255, 255);
        let frame = Rgb(238, 238, 236);
        let panel = Rgb(246, 246, 244);
        Self {
            text: style(text, bg),
            sidebar: style(Rgb(58, 58, 70), panel),
            bar: style(Rgb(59, 59, 62), panel),
            tab: style(Rgb(94, 94, 108), panel),
            tab_active: style(Rgb(51, 51, 51), bg),
            gutter: style(Rgb(102, 102, 110), bg),
            selection: Rgb(173, 214, 255),
            found: Rgb(255, 236, 170),
            row_hover: Rgb(236, 236, 234),
            page_hover: Rgb(243, 243, 245),
            popup_hover: Rgb(238, 240, 246),
            row_active: Rgb(226, 226, 224),
            accent: Rgb(47, 91, 196),
            tint: Rgb(226, 233, 248),
            accept_tint: Rgb(218, 240, 226),
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
            activity: style(Rgb(94, 94, 108), frame),
            status: style(Rgb(94, 94, 108), frame),
            workspaces: style(Rgb(94, 94, 108), frame),
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
            minimap_view: Rgb(222, 226, 238),
            border: Rgb(229, 229, 229),
            focus: Rgb(0, 144, 241),
            popup: Rgb(255, 255, 255),
            popup_dim: Rgb(94, 94, 108),
            disabled: Rgb(150, 150, 160),
            ignored: Rgb(124, 127, 147),
            popup_border: Rgb(214, 214, 214),
            pick: Rgb(214, 226, 252),
            button: Rgb(47, 91, 196),
            button_text: Rgb(255, 255, 255),
        }
    }

    /// A button that isn't the main one, as a wash over whatever's behind it.
    pub fn quiet_button(&self, behind: Rgb) -> Rgb {
        const WASH: i16 = 14;
        let lighter = behind.0 < 128;
        let by = if lighter { WASH } else { -WASH };
        shifted(behind, (by, by, by))
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
        let bg = Rgb(43, 43, 66);
        let frame = Rgb(30, 30, 46);
        let panel = Rgb(36, 36, 56);
        Self {
            text: style(Rgb(214, 214, 226), bg),
            sidebar: style(Rgb(201, 201, 216), panel),
            bar: style(Rgb(201, 201, 216), panel),
            tab: style(Rgb(154, 154, 178), panel),
            tab_active: style(Rgb(255, 255, 255), bg),
            gutter: style(Rgb(154, 154, 178), bg),
            selection: Rgb(62, 80, 130),
            found: Rgb(106, 84, 28),
            row_hover: Rgb(46, 46, 68),
            page_hover: Rgb(53, 53, 80),
            popup_hover: Rgb(66, 66, 106),
            row_active: Rgb(52, 52, 78),
            accent: Rgb(143, 180, 255),
            tint: Rgb(52, 60, 94),
            accept_tint: Rgb(44, 66, 66),
            attention: Rgb(240, 180, 90),
            keyword: Rgb(143, 180, 255),
            string: Rgb(240, 185, 160),
            comment: Rgb(135, 149, 171),
            function: Rgb(232, 217, 160),
            kind: Rgb(127, 214, 194),
            number: Rgb(190, 214, 160),
            added: Rgb(50, 72, 70),
            removed: Rgb(78, 54, 66),
            added_word: Rgb(60, 102, 86),
            removed_word: Rgb(116, 62, 72),
            moved: Rgb(38, 46, 70),
            activity: style(Rgb(127, 132, 156), frame),
            status: style(Rgb(154, 154, 178), frame),
            workspaces: style(Rgb(166, 173, 200), frame),
            breadcrumb: style(Rgb(154, 154, 178), bg),
            line: Rgb(50, 50, 75),
            number_active: Rgb(198, 198, 198),
            guide: Rgb(60, 60, 84),
            scrollbar: Rgb(79, 79, 79),
            error: Rgb(244, 71, 71),
            heading: Rgb(240, 240, 240),
            git_modified: Rgb(240, 180, 90),
            git_added: Rgb(127, 214, 160),
            input: Rgb(30, 30, 46),
            find_match: Rgb(106, 84, 28),
            code_bg: Rgb(34, 34, 52),
            minimap_view: Rgb(72, 74, 110),
            border: Rgb(56, 56, 80),
            focus: Rgb(143, 180, 255),
            popup: Rgb(55, 55, 90),
            popup_dim: Rgb(168, 168, 190),
            disabled: Rgb(122, 122, 150),
            ignored: Rgb(122, 122, 150),
            popup_border: Rgb(78, 78, 112),
            pick: Rgb(72, 80, 128),
            button: Rgb(143, 180, 255),
            button_text: Rgb(20, 20, 31),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_the_terminal_background() {
        let blue = Rgb(30, 30, 46);
        let theme = Theme::default().on_background(blue);
        assert_eq!(
            theme.activity.bg, blue,
            "so no seam shows at the window's edge"
        );
        assert_eq!(theme.status.bg, blue);
        assert_ne!(
            theme.sidebar.bg, blue,
            "the rail stands apart from the sidebar"
        );
        assert_ne!(
            theme.text.bg, theme.sidebar.bg,
            "and the sidebar from the text"
        );
    }

    #[test]
    fn the_surfaces_keep_their_order_on_any_terminal() {
        let brown = Rgb(60, 40, 30);
        let theme = Theme::default().on_background(brown);
        assert!(brightness(theme.sidebar.bg) > brightness(theme.activity.bg));
        assert!(brightness(theme.text.bg) > brightness(theme.sidebar.bg));
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
            // The one showing is the text colour on its chip.
            let chip = brightness(theme.tint) as i32;
            assert!((brightness(theme.text.fg) as i32 - chip).abs() > 120);
            assert!((brightness(theme.activity.fg) as i32 - bg).abs() > 60);
        }
    }
}
