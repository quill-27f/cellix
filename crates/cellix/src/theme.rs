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

    /// An inlay hint's text and chip on a line of colour `under`: the chip a little toward the
    /// text, so it shows on the current line's highlight and in light themes too.
    pub fn hint(&self, under: Rgb) -> (Rgb, Rgb) {
        let toward = |from: u8, to: u8| {
            let (from, to) = (f32::from(from), f32::from(to));
            (from + (to - from) * 0.08).round() as u8
        };
        let Rgb(r, g, b) = under;
        let Rgb(tr, tg, tb) = self.text.fg;
        (
            self.popup_dim,
            Rgb(toward(r, tr), toward(g, tg), toward(b, tb)),
        )
    }

    pub fn is_light(&self) -> bool {
        let Rgb(r, g, b) = self.text.bg;
        u16::from(r) + u16::from(g) + u16::from(b) > 3 * 128
    }

    /// Moves every surface by the same amount, so the panels keep their distance from the
    /// terminal's background. The frame sits exactly on it, light or dark, so the window's
    /// padding, which is the terminal's background, can't show as a seam around the rail and
    /// the status bar. Light, that puts the rest a step or two paler than Latte on a Latte
    /// terminal; the seam was the worse of the two (decided with the user on 2026-11-09).
    /// Text colours stay, since they're chosen for contrast and a nearby background doesn't
    /// change that.
    pub fn on_background(mut self, background: Rgb) -> Self {
        let base = self.activity.bg;
        let by = (
            i16::from(background.0) - i16::from(base.0),
            i16::from(background.1) - i16::from(base.1),
            i16::from(background.2) - i16::from(base.2),
        );
        // The rail and the status bar are the terminal's own colour, so its window padding
        // can't show as a seam around them.
        let (text_was, sidebar_was, popup_was) = (self.text.bg, self.sidebar.bg, self.popup);
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
        self.popup = shifted(self.popup, by);
        // What's drawn on a surface moves as far as that surface did. A light surface stops at
        // white, and a wash moved further would fade into it: a menu's highlight, a text box,
        // a code block on the editor.
        let moved = |was: Rgb, now: Rgb| {
            (
                i16::from(now.0) - i16::from(was.0),
                i16::from(now.1) - i16::from(was.1),
                i16::from(now.2) - i16::from(was.2),
            )
        };
        let on_text = moved(text_was, self.text.bg);
        let on_sidebar = moved(sidebar_was, self.sidebar.bg);
        let on_popup = moved(popup_was, self.popup);
        for color in [
            &mut self.page_hover,
            &mut self.line,
            &mut self.guide,
            &mut self.scrollbar,
            &mut self.code_bg,
            &mut self.minimap_view,
            &mut self.border,
            &mut self.tint,
            &mut self.accept_tint,
        ] {
            *color = shifted(*color, on_text);
        }
        for color in [&mut self.row_hover, &mut self.row_active] {
            *color = shifted(*color, on_sidebar);
        }
        for color in [
            &mut self.popup_hover,
            &mut self.input,
            &mut self.popup_border,
            &mut self.pick,
        ] {
            *color = shifted(*color, on_popup);
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

    /// Catppuccin Latte's surfaces: the editor on base, the sidebars on mantle, the frame on
    /// crust, and menus white. Its text and code colours are deepened until each reads at
    /// 4.5 to 1 on those surfaces, which Latte's own green, yellow and blue don't.
    fn light() -> Self {
        let text = Rgb(0x4C, 0x4F, 0x69);
        let second = Rgb(0x53, 0x56, 0x6F);
        let third = Rgb(0x5C, 0x5F, 0x77);
        let base = Rgb(0xEF, 0xF1, 0xF5);
        let mantle = Rgb(0xE6, 0xE9, 0xEF);
        let crust = Rgb(0xDC, 0xE0, 0xE8);
        let surface0 = Rgb(0xCC, 0xD0, 0xDA);
        let surface1 = Rgb(0xBC, 0xC0, 0xCC);
        let line = Rgb(0xAC, 0xB0, 0xBE);
        let overlay0 = Rgb(0x9C, 0xA0, 0xB0);
        let overlay2 = Rgb(0x7C, 0x7F, 0x93);
        let blue = Rgb(0x1A, 0x5A, 0xE0);
        let mauve = Rgb(0x7C, 0x3A, 0xED);
        let green = Rgb(0x1F, 0x7A, 0x1A);
        // Washes keep Latte's own green, yellow and red: they sit behind text, not as it.
        let wash_green = Rgb(0x40, 0xA0, 0x2B);
        let wash_yellow = Rgb(0xDF, 0x8E, 0x1D);
        let wash_red = Rgb(0xD2, 0x0F, 0x39);
        let amber = Rgb(0x8A, 0x5A, 0x00);
        let peach = Rgb(0xB3, 0x4A, 0x05);
        let red = Rgb(0xC2, 0x0E, 0x35);
        let lavender = Rgb(0x72, 0x87, 0xFD);
        let white = Rgb(0xFF, 0xFF, 0xFF);
        let chosen = Rgb(0xC8, 0xD6, 0xF8);
        Self {
            text: style(text, base),
            sidebar: style(text, mantle),
            bar: style(text, mantle),
            tab: style(second, crust),
            tab_active: style(text, base),
            gutter: style(third, base),
            selection: chosen,
            found: mix(base, wash_yellow, 0.3),
            row_hover: crust,
            page_hover: mantle,
            popup_hover: mantle,
            row_active: chosen,
            accent: blue,
            tint: mix(base, blue, 0.13),
            accept_tint: mix(base, wash_green, 0.15),
            attention: peach,
            keyword: mauve,
            string: green,
            comment: third,
            function: blue,
            kind: amber,
            number: peach,
            added: mix(base, wash_green, 0.15),
            removed: mix(base, wash_red, 0.15),
            added_word: mix(base, wash_green, 0.3),
            removed_word: mix(base, wash_red, 0.3),
            moved: mix(base, lavender, 0.15),
            activity: style(third, crust),
            status: style(second, crust),
            workspaces: style(second, crust),
            breadcrumb: style(second, base),
            line: mantle,
            number_active: text,
            guide: surface0,
            scrollbar: surface1,
            error: red,
            heading: text,
            git_modified: amber,
            git_added: green,
            input: white,
            find_match: mix(base, wash_yellow, 0.45),
            code_bg: mantle,
            minimap_view: crust,
            border: surface0,
            focus: blue,
            popup: white,
            popup_dim: second,
            disabled: overlay0,
            ignored: overlay2,
            popup_border: line,
            pick: mix(white, blue, 0.16),
            button: blue,
            button_text: white,
        }
    }

    /// A button that isn't the main one, as a wash over whatever's behind it.
    pub fn quiet_button(&self, behind: Rgb) -> Rgb {
        // Light grounds need a deeper wash to read as a button at all: Latte's base and a
        // 14 step apart are 1.14:1, under the 1.3:1 a quiet button needs.
        const WASH: i16 = 14;
        const LIGHT_WASH: i16 = 24;
        let lighter = behind.0 < 128;
        let by = if lighter { WASH } else { -LIGHT_WASH };
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
    /// The dark theme: three surfaces a step apart (the frame, the panels, the sheet), three
    /// weights of text, one blue, and Catppuccin Mocha's colours for code and for meaning.
    fn default() -> Self {
        let sheet = Rgb(43, 43, 66);
        let frame = Rgb(30, 30, 46);
        let panel = Rgb(36, 36, 56);
        let raised = Rgb(51, 51, 77);
        let text = Rgb(205, 214, 244);
        let second = Rgb(166, 173, 200);
        let third = Rgb(147, 153, 178);
        let line = Rgb(58, 58, 86);
        let strong_line = Rgb(80, 80, 111);
        let blue = Rgb(137, 180, 250);
        let chosen = Rgb(57, 66, 107);
        let green = Rgb(166, 227, 161);
        let peach = Rgb(250, 179, 135);
        let red = Rgb(243, 139, 168);
        let yellow = Rgb(249, 226, 175);
        let mauve = Rgb(203, 166, 247);
        // What's under the pointer: the text's colour, an eighth of the way over its ground.
        let hover = |ground: Rgb| mix(ground, text, 0.08);
        Self {
            text: style(text, sheet),
            sidebar: style(text, panel),
            bar: style(text, panel),
            tab: style(second, frame),
            tab_active: style(text, sheet),
            gutter: style(third, sheet),
            selection: chosen,
            found: mix(sheet, yellow, 0.28),
            row_hover: hover(panel),
            page_hover: hover(sheet),
            popup_hover: hover(raised),
            row_active: chosen,
            accent: blue,
            tint: Rgb(54, 60, 92),
            accept_tint: Rgb(44, 66, 66),
            attention: peach,
            keyword: mauve,
            string: green,
            comment: third,
            function: blue,
            kind: yellow,
            number: peach,
            // Deeper than a wash of Mocha's green and red, which on this blue sheet came
            // out grey: a line added has to read as green at a glance.
            added: Rgb(50, 72, 70),
            removed: Rgb(78, 54, 66),
            added_word: Rgb(60, 102, 86),
            removed_word: Rgb(116, 62, 72),
            moved: Rgb(38, 46, 70),
            activity: style(third, frame),
            status: style(second, frame),
            workspaces: style(second, frame),
            breadcrumb: style(second, sheet),
            line: hover(sheet),
            number_active: text,
            guide: line,
            scrollbar: strong_line,
            error: red,
            heading: text,
            git_modified: peach,
            git_added: green,
            input: Rgb(28, 28, 42),
            find_match: mix(sheet, yellow, 0.45),
            code_bg: Rgb(34, 34, 52),
            minimap_view: Rgb(72, 74, 110),
            border: line,
            focus: blue,
            popup: raised,
            popup_dim: second,
            disabled: Rgb(122, 122, 150),
            ignored: Rgb(122, 122, 150),
            popup_border: strong_line,
            pick: chosen,
            button: blue,
            button_text: Rgb(17, 17, 27),
        }
    }
}

/// `by` of the way from `from` to `to`: a wash of one colour over another.
fn mix(from: Rgb, to: Rgb, by: f32) -> Rgb {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * by).round() as u8;
    Rgb(
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Light is Catppuccin Latte's surfaces: base, mantle and crust for the editor, the
    /// sidebars and the frame, and menus white. Its code colours are Latte's hues deepened,
    /// since Latte's own green is 3 to 1 on base and its yellow less.
    #[test]
    fn the_light_theme_is_catppuccin_latte() {
        let latte = Theme::named(Some("light"));
        assert_eq!(latte.text.bg, Rgb(0xEF, 0xF1, 0xF5), "base");
        assert_eq!(latte.sidebar.bg, Rgb(0xE6, 0xE9, 0xEF), "mantle");
        assert_eq!(latte.activity.bg, Rgb(0xDC, 0xE0, 0xE8), "crust");
        assert_eq!(latte.popup, Rgb(0xFF, 0xFF, 0xFF), "menus");
        assert_eq!(latte.text.fg, Rgb(0x4C, 0x4F, 0x69), "Latte's text");
        for (name, color) in [
            ("comment", latte.comment),
            ("keyword", latte.keyword),
            ("string", latte.string),
            ("function", latte.function),
            ("type", latte.kind),
            ("number", latte.number),
            ("quiet text", latte.gutter.fg),
        ] {
            let ratio = contrast(color, latte.text.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on base");
        }
    }

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
    fn a_light_theme_meets_a_light_terminal_without_a_seam() {
        // Latte's base, as a light terminal's background.
        let base = Rgb(0xEF, 0xF1, 0xF5);
        let theme = Theme::named(Some("light")).on_background(base);
        assert_eq!(
            theme.activity.bg, base,
            "the window's padding meets the rail"
        );
        assert_eq!(theme.status.bg, base);
        assert!(brightness(theme.sidebar.bg) > brightness(theme.activity.bg));
        assert!(brightness(theme.text.bg) > brightness(theme.sidebar.bg));
        // Surfaces stop at white; what's drawn on them keeps its step from them.
        let latte = Theme::named(Some("light"));
        assert_eq!(theme.popup, latte.popup, "menus were white already");
        assert_eq!(
            theme.pick, latte.pick,
            "so their highlight stays where it was"
        );
        assert_eq!(theme.popup_hover, latte.popup_hover);
        assert_eq!(theme.input, latte.input);
        // On the editor, code blocks and lines keep about the step they had in Latte.
        let step = |t: &Theme, wash: Rgb| brightness(t.text.bg) as i32 - brightness(wash) as i32;
        for (now, then) in [
            (step(&theme, theme.code_bg), step(&latte, latte.code_bg)),
            (step(&theme, theme.border), step(&latte, latte.border)),
        ] {
            assert!((now - then).abs() <= 3 && now > 0, "{now} against {then}");
        }
    }

    #[test]
    fn the_surfaces_keep_their_order_on_any_terminal() {
        let brown = Rgb(60, 40, 30);
        let theme = Theme::default().on_background(brown);
        assert!(brightness(theme.sidebar.bg) > brightness(theme.activity.bg));
        assert!(brightness(theme.text.bg) > brightness(theme.sidebar.bg));
    }

    /// How far apart two colours are to the eye, as WCAG counts it: 1 to 21.
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let light = |c: Rgb| 0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2);
        let (a, b) = (light(a), light(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The dark theme's text and code read at 4.5 to 1 on the sheet, and its three surfaces
    /// are the design's: the frame, the panels and the sheet.
    #[test]
    fn the_dark_theme_is_three_surfaces_and_text_that_reads_on_them() {
        let dark = Theme::default();
        assert_eq!(dark.activity.bg, Rgb(30, 30, 46), "the frame");
        assert_eq!(dark.sidebar.bg, Rgb(36, 36, 56), "the panels");
        assert_eq!(dark.text.bg, Rgb(43, 43, 66), "the sheet");
        assert_eq!(dark.tab.bg, dark.activity.bg, "tabs stand on the frame");
        assert_eq!(
            dark.tab_active.bg, dark.text.bg,
            "the one in front on its sheet"
        );
        for (name, color) in [
            ("text", dark.text.fg),
            ("comment", dark.comment),
            ("keyword", dark.keyword),
            ("string", dark.string),
            ("function", dark.function),
            ("type", dark.kind),
            ("number", dark.number),
            ("quiet text", dark.gutter.fg),
            ("a failure", dark.error),
        ] {
            let ratio = contrast(color, dark.text.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on the sheet");
        }
        for (name, color) in [
            ("quiet text", dark.status.fg),
            ("an icon", dark.activity.fg),
        ] {
            let ratio = contrast(color, dark.activity.bg);
            assert!(ratio >= 4.5, "{name} is {ratio:.2} to 1 on the frame");
        }
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
